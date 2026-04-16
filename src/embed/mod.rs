use anyhow::{Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use std::path::Path;
use std::sync::Mutex;

pub struct Embedder {
    inner: EmbedderInner,
}

#[allow(clippy::large_enum_variant)]
enum EmbedderInner {
    Candle {
        model: Mutex<BertModel>,
        tokenizer: tokenizers::Tokenizer,
        dims: usize,
        device: Device,
    },
    Unavailable {
        dims: usize,
    },
}

pub struct EmbeddingResult {
    pub vector: Vec<f32>,
    pub token_count: usize,
}

impl Embedder {
    pub fn load(vault_path: &Path) -> Result<Self> {
        let model_dir = vault_path.join("models").join("all-MiniLM-L6-v2");
        let model_path = model_dir.join("model.safetensors");
        let tokenizer_path = model_dir.join("tokenizer.json");
        let config_path = model_dir.join("config.json");

        if !model_path.exists() || !tokenizer_path.exists() || !config_path.exists() {
            tracing::warn!(
                "Embedding model not found at {}. Run ./scripts/download-model.sh",
                model_dir.display()
            );
            return Ok(Embedder {
                inner: EmbedderInner::Unavailable { dims: 384 },
            });
        }

        match Self::load_candle(&model_path, &tokenizer_path, &config_path) {
            Ok(inner) => {
                tracing::info!("Loaded embedding model: all-MiniLM-L6-v2 (candle)");
                Ok(Embedder { inner })
            }
            Err(e) => {
                tracing::warn!("Failed to load model: {e}");
                Ok(Embedder {
                    inner: EmbedderInner::Unavailable { dims: 384 },
                })
            }
        }
    }

    fn load_candle(
        model_path: &Path,
        tokenizer_path: &Path,
        config_path: &Path,
    ) -> Result<EmbedderInner> {
        let device = Device::Cpu;

        let config_str =
            std::fs::read_to_string(config_path).context("Failed to read config.json")?;
        let config: Config =
            serde_json::from_str(&config_str).context("Failed to parse config.json")?;

        let dims = config.hidden_size;

        // SAFETY: The safetensors file is read-only and not modified while mapped.
        // Memory-mapping is safe as long as the file is not truncated externally.
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[model_path], DType::F32, &device)
                .context("Failed to load model weights")?
        };

        let model = BertModel::load(vb, &config).context("Failed to build BERT model")?;

        let tokenizer = tokenizers::Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {e}"))?;

        Ok(EmbedderInner::Candle {
            model: Mutex::new(model),
            tokenizer,
            dims,
            device,
        })
    }

    pub fn embed(&self, text: &str) -> Result<Vec<f32>> {
        Ok(self.embed_with_info(text)?.vector)
    }

    pub fn embed_with_info(&self, text: &str) -> Result<EmbeddingResult> {
        match &self.inner {
            EmbedderInner::Candle {
                model,
                tokenizer,
                dims,
                device,
            } => {
                let mut model = model.lock().unwrap();
                Self::run_inference(&mut model, tokenizer, *dims, device, text)
            }
            EmbedderInner::Unavailable { .. } => {
                anyhow::bail!(
                    "Embedding model not loaded. Download it with ./scripts/download-model.sh"
                )
            }
        }
    }

    fn run_inference(
        model: &mut BertModel,
        tokenizer: &tokenizers::Tokenizer,
        dims: usize,
        device: &Device,
        text: &str,
    ) -> Result<EmbeddingResult> {
        let encoding = tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("Tokenization failed: {e}"))?;

        let input_ids: Vec<u32> = encoding.get_ids().to_vec();
        let attention_mask: Vec<u32> = encoding.get_attention_mask().to_vec();
        let token_type_ids: Vec<u32> = encoding.get_type_ids().to_vec();
        let seq_len = input_ids.len();

        let input_ids_t = Tensor::new(&input_ids[..], device)?.unsqueeze(0)?;
        let attention_mask_t = Tensor::new(&attention_mask[..], device)?.unsqueeze(0)?;
        let token_type_ids_t = Tensor::new(&token_type_ids[..], device)?.unsqueeze(0)?;

        let output = model.forward(&input_ids_t, &token_type_ids_t, Some(&attention_mask_t))?;

        // Mean pooling: average token embeddings weighted by attention mask
        let attention_f = attention_mask_t.to_dtype(DType::F32)?.unsqueeze(2)?;
        let weighted = output.broadcast_mul(&attention_f)?;
        let summed = weighted.sum(1)?;
        let mask_sum = attention_f.sum(1)?;
        let pooled = summed.broadcast_div(&mask_sum)?;

        // L2 normalize
        let norm = pooled.sqr()?.sum(1)?.sqrt()?;
        let normalized = pooled.broadcast_div(&norm.unsqueeze(1)?)?;

        let embedding: Vec<f32> = normalized.squeeze(0)?.to_vec1()?;

        Ok(EmbeddingResult {
            vector: embedding,
            token_count: seq_len,
        })
    }

    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        texts.iter().map(|t| self.embed(t)).collect()
    }

    pub fn dims(&self) -> usize {
        match &self.inner {
            EmbedderInner::Candle { dims, .. } | EmbedderInner::Unavailable { dims } => *dims,
        }
    }

    pub fn is_available(&self) -> bool {
        matches!(&self.inner, EmbedderInner::Candle { .. })
    }

    pub fn model_name(&self) -> &str {
        "all-MiniLM-L6-v2"
    }
}
