use anyhow::Result;

use super::{ChunkVc, ChunkableAsset, ChunkableAssetVc, ChunkingContext, ChunkingContextVc};
use crate::asset::{Asset, AssetVc, AssetsVc};

/// Marker trait for the chunking context to accept evaluated entries.
///
/// The chunking context implementation will resolve the dynamic entry to a
/// well-known value or trait object.
#[turbo_tasks::value_trait]
pub trait EvaluatableAsset: Asset + ChunkableAsset {}

#[turbo_tasks::value(transparent)]
pub struct EvaluatableAssets(Vec<EvaluatableAssetVc>);

#[turbo_tasks::value_impl]
impl EvaluatableAssetsVc {
    #[turbo_tasks::function]
    pub fn empty() -> EvaluatableAssetsVc {
        EvaluatableAssets(vec![]).cell()
    }

    #[turbo_tasks::function]
    pub fn one(entry: EvaluatableAssetVc) -> EvaluatableAssetsVc {
        EvaluatableAssets(vec![entry]).cell()
    }

    #[turbo_tasks::function]
    pub async fn with_entry(self, entry: EvaluatableAssetVc) -> Result<EvaluatableAssetsVc> {
        let mut entries = self.await?.clone_value();
        entries.push(entry);
        Ok(EvaluatableAssets(entries).cell())
    }
}

/// Trait for chunking contexts which can generate evaluated chunks.
#[turbo_tasks::value_trait]
pub trait EvaluateChunkingContext: ChunkingContext {
    /// Create a chunk that evaluates the given entries.
    fn evaluate_chunk(
        &self,
        entry_chunk: ChunkVc,
        other_assets: AssetsVc,
        evaluatable_assets: EvaluatableAssetsVc,
    ) -> AssetVc;
}
