use anyhow::{Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use std::path::Path;
use std::sync::Mutex;

// ─────────────────────────────────────────────────────────────────────────────
// Model registry
//
// Default: BAAI/bge-small-en-v1.5 — 33M params, 384-d, MTEB ~62 (~5pt above
// MiniLM-L6-v2). Same BERT architecture, same dim, candle-supported.
//
// Fallback: sentence-transformers/all-MiniLM-L6-v2 — 22M params, 384-d.
// Older but still loadable; kept so vaults that already pulled MiniLM keep
// working without forcing a re-index.
// ─────────────────────────────────────────────────────────────────────────────

const PRIMARY_MODEL: &str = "bge-small-en-v1.5";
const FALLBACK_MODEL: &str = "all-MiniLM-L6-v2";

#[cfg(feature = "embed-model")]
mod embedded {
    pub const MODEL: &[u8] =
        include_bytes!("../../vault/models/bge-small-en-v1.5/model.safetensors");
    pub const TOKENIZER: &[u8] =
        include_bytes!("../../vault/models/bge-small-en-v1.5/tokenizer.json");
    pub const CONFIG: &[u8] = include_bytes!("../../vault/models/bge-small-en-v1.5/config.json");
}

/// How to collapse the per-token hidden states into a single embedding.
#[derive(Clone, Copy)]
enum Pooling {
    /// Average non-padding tokens. Default for `all-MiniLM-L6-v2`.
    Mean,
    /// Take the [CLS] token's hidden state. Default for the BGE family —
    /// using mean pooling for BGE drops retrieval quality noticeably.
    Cls,
}

impl Pooling {
    fn for_model(name: &str) -> Self {
        // BGE / Snowflake Arctic / e5 all expect CLS pooling.
        if name.starts_with("bge-")
            || name.starts_with("snowflake-arctic")
            || name.starts_with("multilingual-e5")
            || name.starts_with("e5-")
        {
            Pooling::Cls
        } else {
            Pooling::Mean
        }
    }
}

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
        name: String,
        pooling: Pooling,
    },
    Unavailable {
        dims: usize,
    },
    #[cfg(feature = "test-support")]
    Fixed {
        dims: usize,
        vector: Vec<f32>,
    },
}

pub struct EmbeddingResult {
    pub vector: Vec<f32>,
    pub token_count: usize,
}

impl Embedder {
    pub fn load(vault_path: &Path) -> Result<Self> {
        let models_root = vault_path.join("models");

        // Probe disk in preference order. Either lives at
        // `<vault>/models/<name>/{model.safetensors,tokenizer.json,config.json}`.
        let on_disk: Vec<&str> = [PRIMARY_MODEL, FALLBACK_MODEL]
            .into_iter()
            .filter(|name| {
                let dir = models_root.join(name);
                dir.join("model.safetensors").exists()
                    && dir.join("tokenizer.json").exists()
                    && dir.join("config.json").exists()
            })
            .collect();

        if on_disk.len() > 1 {
            tracing::warn!(
                "Multiple embedding models on disk ({:?}); preferring '{}'. \
                 If your DB was indexed with a different model, results will be \
                 inaccurate — re-ingest after removing the unused model directory.",
                on_disk,
                on_disk[0]
            );
        }

        for name in on_disk {
            let dir = models_root.join(name);
            let model_path = dir.join("model.safetensors");
            let tokenizer_path = dir.join("tokenizer.json");
            let config_path = dir.join("config.json");
            match Self::load_from_files(&model_path, &tokenizer_path, &config_path, name) {
                Ok(emb) => {
                    tracing::info!("Loaded embedding model from disk: {name}");
                    return Ok(emb);
                }
                Err(e) => {
                    tracing::warn!("Failed to load model {name} from disk: {e}");
                }
            }
        }

        // Bundled bytes (release builds with `--features embed-model`).
        #[cfg(feature = "embed-model")]
        {
            match Self::load_from_bytes(
                embedded::MODEL,
                embedded::TOKENIZER,
                embedded::CONFIG,
                PRIMARY_MODEL,
            ) {
                Ok(emb) => {
                    tracing::info!("Loaded embedded model: {PRIMARY_MODEL}");
                    return Ok(emb);
                }
                Err(e) => {
                    tracing::warn!("Failed to load embedded model: {e}");
                }
            }
        }

        tracing::warn!(
            "No embedding model available. Run ./scripts/download-model.sh \
             or build with --features embed-model"
        );
        Ok(Embedder {
            inner: EmbedderInner::Unavailable { dims: 384 },
        })
    }

    fn load_from_files(
        model_path: &Path,
        tokenizer_path: &Path,
        config_path: &Path,
        name: &str,
    ) -> Result<Self> {
        let device = Device::Cpu;

        let config_str =
            std::fs::read_to_string(config_path).context("Failed to read config.json")?;
        let config: Config =
            serde_json::from_str(&config_str).context("Failed to parse config.json")?;
        let dims = config.hidden_size;

        // SAFETY: the safetensors file is read-only and not modified while mapped.
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[model_path], DType::F32, &device)
                .context("Failed to load model weights")?
        };

        let model = build_bert(vb, &config)?;
        let tokenizer = tokenizers::Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {e}"))?;

        Ok(Embedder {
            inner: EmbedderInner::Candle {
                model: Mutex::new(model),
                tokenizer,
                dims,
                device,
                name: name.to_string(),
                pooling: Pooling::for_model(name),
            },
        })
    }

    #[cfg(feature = "embed-model")]
    fn load_from_bytes(
        model_bytes: &[u8],
        tokenizer_bytes: &[u8],
        config_bytes: &[u8],
        name: &str,
    ) -> Result<Self> {
        let device = Device::Cpu;

        let config: Config =
            serde_json::from_slice(config_bytes).context("Failed to parse embedded config")?;
        let dims = config.hidden_size;

        let vb = VarBuilder::from_buffered_safetensors(model_bytes.to_vec(), DType::F32, &device)
            .context("Failed to load embedded model weights")?;

        let model = build_bert(vb, &config)?;
        let tokenizer = tokenizers::Tokenizer::from_bytes(tokenizer_bytes)
            .map_err(|e| anyhow::anyhow!("Failed to load embedded tokenizer: {e}"))?;

        Ok(Embedder {
            inner: EmbedderInner::Candle {
                model: Mutex::new(model),
                tokenizer,
                dims,
                device,
                name: name.to_string(),
                pooling: Pooling::for_model(name),
            },
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
                pooling,
                ..
            } => {
                let mut model = model.lock().unwrap();
                Self::run_inference(&mut model, tokenizer, *dims, device, *pooling, text)
            }
            EmbedderInner::Unavailable { .. } => {
                anyhow::bail!(
                    "Embedding model not loaded. Download it with ./scripts/download-model.sh"
                )
            }
            #[cfg(feature = "test-support")]
            EmbedderInner::Fixed { vector, .. } => Ok(EmbeddingResult {
                vector: vector.clone(),
                token_count: text.split_whitespace().count(),
            }),
        }
    }

    #[cfg(feature = "test-support")]
    pub fn fixed(dims: usize) -> Self {
        let mut vector = vec![0.0f32; dims];
        vector[0] = 1.0;
        Embedder {
            inner: EmbedderInner::Fixed { dims, vector },
        }
    }

    #[cfg(feature = "test-support")]
    pub fn unavailable() -> Self {
        Embedder {
            inner: EmbedderInner::Unavailable { dims: 384 },
        }
    }

    fn run_inference(
        model: &mut BertModel,
        tokenizer: &tokenizers::Tokenizer,
        _dims: usize,
        device: &Device,
        pooling: Pooling,
        text: &str,
    ) -> Result<EmbeddingResult> {
        let encoding = tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("Tokenization failed: {e}"))?;

        // Both BGE-small-en-v1.5 and all-MiniLM-L6-v2 ship a BERT config with
        // `max_position_embeddings: 512`. The tokenizer.json may or may not
        // configure truncation; long inputs (mbox emails, big PDFs) blow past
        // 512 tokens and `index-select invalid index 512 with dim size 512`
        // surfaces from the position-embedding lookup. Hard-cap the slice
        // here so every model in the registry stays within bounds.
        const MAX_SEQ_LEN: usize = 512;
        // BERT uncased WordPiece [SEP] = 102 — same in BGE/MiniLM. Used to
        // cap a truncated sequence so the model still sees a sentence end.
        const SEP_TOKEN_ID: u32 = 102;

        let mut input_ids: Vec<u32> = encoding.get_ids().to_vec();
        let mut attention_mask: Vec<u32> = encoding.get_attention_mask().to_vec();
        let mut token_type_ids: Vec<u32> = encoding.get_type_ids().to_vec();
        if input_ids.len() > MAX_SEQ_LEN {
            input_ids.truncate(MAX_SEQ_LEN);
            attention_mask.truncate(MAX_SEQ_LEN);
            token_type_ids.truncate(MAX_SEQ_LEN);
            // Replace the final WordPiece with [SEP] so the model still
            // sees a closing-sentence token rather than a mid-word fragment.
            if let Some(last) = input_ids.last_mut() {
                *last = SEP_TOKEN_ID;
            }
        }
        let seq_len = input_ids.len();

        let input_ids_t = Tensor::new(&input_ids[..], device)?.unsqueeze(0)?;
        let attention_mask_t = Tensor::new(&attention_mask[..], device)?.unsqueeze(0)?;
        let token_type_ids_t = Tensor::new(&token_type_ids[..], device)?.unsqueeze(0)?;

        let output = model.forward(&input_ids_t, &token_type_ids_t, Some(&attention_mask_t))?;

        let pooled = match pooling {
            Pooling::Mean => {
                let attention_f = attention_mask_t.to_dtype(DType::F32)?.unsqueeze(2)?;
                let weighted = output.broadcast_mul(&attention_f)?;
                let summed = weighted.sum(1)?;
                let mask_sum = attention_f.sum(1)?;
                summed.broadcast_div(&mask_sum)?
            }
            Pooling::Cls => {
                // [batch, seq, hidden] -> [batch, hidden] using token 0 ([CLS]).
                output.i((.., 0))?
            }
        };

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
            #[cfg(feature = "test-support")]
            EmbedderInner::Fixed { dims, .. } => *dims,
        }
    }

    pub fn is_available(&self) -> bool {
        match &self.inner {
            EmbedderInner::Candle { .. } => true,
            #[cfg(feature = "test-support")]
            EmbedderInner::Fixed { .. } => true,
            _ => false,
        }
    }

    pub fn model_name(&self) -> &str {
        match &self.inner {
            EmbedderInner::Candle { name, .. } => name.as_str(),
            #[cfg(feature = "test-support")]
            EmbedderInner::Fixed { .. } => PRIMARY_MODEL,
            _ => PRIMARY_MODEL,
        }
    }
}

// `IndexOp` brings `.i((..., 0))` slicing into scope.
use candle_core::IndexOp;

/// Try loading a `BertModel` directly first, then with a `bert.` prefix.
/// `BAAI/bge-*` ships with no prefix (saved as `BertModel`), but some
/// derivatives keep the `bert.` namespace from `BertForXxx` checkpoints.
fn build_bert(vb: VarBuilder, config: &Config) -> Result<BertModel> {
    match BertModel::load(vb.clone(), config) {
        Ok(m) => Ok(m),
        Err(first) => {
            // Retry under the "bert" submodule. The error from BertModel::load
            // when keys are missing is verbose; surface the original message
            // only if both attempts fail so the user gets the meaningful one.
            match BertModel::load(vb.pp("bert"), config) {
                Ok(m) => Ok(m),
                Err(_) => Err(anyhow::anyhow!(
                    "Failed to load BERT weights (no prefix and bert.* both rejected): {first}"
                )),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixed_embedder_returns_vector() {
        let embedder = Embedder::fixed(384);
        let result = embedder.embed("hello world").unwrap();
        assert_eq!(result.len(), 384);
    }

    #[test]
    fn test_fixed_embedder_deterministic() {
        let embedder = Embedder::fixed(384);
        let a = embedder.embed("test").unwrap();
        let b = embedder.embed("test").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_fixed_embedder_dims() {
        let embedder = Embedder::fixed(384);
        assert_eq!(embedder.dims(), 384);
    }

    #[test]
    fn test_fixed_embedder_is_available() {
        let embedder = Embedder::fixed(384);
        assert!(embedder.is_available());
    }

    #[test]
    fn test_fixed_embedder_model_name() {
        let embedder = Embedder::fixed(384);
        assert_eq!(embedder.model_name(), PRIMARY_MODEL);
    }

    #[test]
    fn test_unavailable_embedder_fails() {
        let embedder = Embedder::unavailable();
        assert!(!embedder.is_available());
        assert!(embedder.embed("hello").is_err());
    }

    #[test]
    fn test_embed_batch_fixed() {
        let embedder = Embedder::fixed(384);
        let results = embedder.embed_batch(&["one", "two", "three"]).unwrap();
        assert_eq!(results.len(), 3);
        for v in &results {
            assert_eq!(v.len(), 384);
        }
    }

    #[test]
    fn test_pooling_for_model() {
        assert!(matches!(
            Pooling::for_model("bge-small-en-v1.5"),
            Pooling::Cls
        ));
        assert!(matches!(
            Pooling::for_model("snowflake-arctic-embed-s"),
            Pooling::Cls
        ));
        assert!(matches!(
            Pooling::for_model("all-MiniLM-L6-v2"),
            Pooling::Mean
        ));
    }
}
