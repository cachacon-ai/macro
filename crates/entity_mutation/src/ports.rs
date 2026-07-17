use async_trait::async_trait;

use crate::{
    DuplicateEntityRequest, EntityMutationActor, EntityMutationOutcome, EntityRef,
    MoveEntityRequest, RenameEntityRequest, UpdateEntitySharePolicyRequest,
};

/// Domain port used by API adapters to mutate heterogeneous entities.
///
/// Methods are batch-oriented because the existing UI performs these actions
/// across selections. Each input always produces one ordered outcome, so
/// cross-service partial success is explicit and no false transaction boundary
/// is implied.
#[async_trait]
pub trait EntityMutationService: Send + Sync + 'static {
    /// Rename entities using the canonical `display_name` concept.
    async fn rename_entities(
        &self,
        actor: EntityMutationActor,
        requests: Vec<RenameEntityRequest>,
    ) -> Vec<EntityMutationOutcome>;

    /// Move entities into a project, or to the root when `project_id` is absent.
    async fn move_entities(
        &self,
        actor: EntityMutationActor,
        requests: Vec<MoveEntityRequest>,
    ) -> Vec<EntityMutationOutcome>;

    /// Update public and channel share policies.
    async fn update_share_policies(
        &self,
        actor: EntityMutationActor,
        requests: Vec<UpdateEntitySharePolicyRequest>,
    ) -> Vec<EntityMutationOutcome>;

    /// Soft-delete entities that support a reversible trash lifecycle.
    async fn trash_entities(
        &self,
        actor: EntityMutationActor,
        entities: Vec<EntityRef>,
    ) -> Vec<EntityMutationOutcome>;

    /// Restore reversibly deleted entities.
    async fn restore_entities(
        &self,
        actor: EntityMutationActor,
        entities: Vec<EntityRef>,
    ) -> Vec<EntityMutationOutcome>;

    /// Irreversibly delete entities.
    async fn delete_entities_permanently(
        &self,
        actor: EntityMutationActor,
        entities: Vec<EntityRef>,
    ) -> Vec<EntityMutationOutcome>;

    /// Duplicate entities that support copy semantics.
    async fn duplicate_entities(
        &self,
        actor: EntityMutationActor,
        requests: Vec<DuplicateEntityRequest>,
    ) -> Vec<EntityMutationOutcome>;

    /// Add or remove an entity from the actor's favorites.
    async fn set_favorite(
        &self,
        actor: EntityMutationActor,
        entity: EntityRef,
        favorite: bool,
    ) -> EntityMutationOutcome;
}

/// Schema-only implementation used when no mutation services are wired.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnavailableEntityMutationService;

fn unsupported_many(
    operation: &str,
    entities: impl IntoIterator<Item = EntityRef>,
) -> Vec<EntityMutationOutcome> {
    entities
        .into_iter()
        .map(|entity| EntityMutationOutcome::unsupported(entity, operation))
        .collect()
}

#[async_trait]
impl EntityMutationService for UnavailableEntityMutationService {
    async fn rename_entities(
        &self,
        _actor: EntityMutationActor,
        requests: Vec<RenameEntityRequest>,
    ) -> Vec<EntityMutationOutcome> {
        unsupported_many("rename", requests.into_iter().map(|request| request.entity))
    }

    async fn move_entities(
        &self,
        _actor: EntityMutationActor,
        requests: Vec<MoveEntityRequest>,
    ) -> Vec<EntityMutationOutcome> {
        unsupported_many("move", requests.into_iter().map(|request| request.entity))
    }

    async fn update_share_policies(
        &self,
        _actor: EntityMutationActor,
        requests: Vec<UpdateEntitySharePolicyRequest>,
    ) -> Vec<EntityMutationOutcome> {
        unsupported_many(
            "share policy updates",
            requests.into_iter().map(|request| request.entity),
        )
    }

    async fn trash_entities(
        &self,
        _actor: EntityMutationActor,
        entities: Vec<EntityRef>,
    ) -> Vec<EntityMutationOutcome> {
        unsupported_many("trash", entities)
    }

    async fn restore_entities(
        &self,
        _actor: EntityMutationActor,
        entities: Vec<EntityRef>,
    ) -> Vec<EntityMutationOutcome> {
        unsupported_many("restore", entities)
    }

    async fn delete_entities_permanently(
        &self,
        _actor: EntityMutationActor,
        entities: Vec<EntityRef>,
    ) -> Vec<EntityMutationOutcome> {
        unsupported_many("permanent deletion", entities)
    }

    async fn duplicate_entities(
        &self,
        _actor: EntityMutationActor,
        requests: Vec<DuplicateEntityRequest>,
    ) -> Vec<EntityMutationOutcome> {
        unsupported_many(
            "duplication",
            requests.into_iter().map(|request| request.entity),
        )
    }

    async fn set_favorite(
        &self,
        _actor: EntityMutationActor,
        entity: EntityRef,
        _favorite: bool,
    ) -> EntityMutationOutcome {
        EntityMutationOutcome::unsupported(entity, "favorites")
    }
}
