use macro_user_id::user_id::MacroUserIdStr;
use model_entity::EntityType;
use models_permissions::share_permission::UpdateSharePermissionRequestV2;

/// Authenticated actor performing an entity mutation.
#[derive(Clone, Debug)]
pub struct EntityMutationActor {
    /// Stable Macro user id.
    pub user_id: MacroUserIdStr<'static>,
    /// Organization id attached to the authenticated request, when present.
    pub organization_id: Option<i64>,
}

/// Canonical reference to an entity.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EntityRef {
    /// Entity kind.
    pub entity_type: EntityType,
    /// Entity identifier in the canonical namespace for its kind.
    pub entity_id: String,
}

impl EntityRef {
    /// Create a canonical entity reference.
    pub fn new(entity_type: EntityType, entity_id: impl Into<String>) -> Self {
        Self {
            entity_type,
            entity_id: entity_id.into(),
        }
    }
}

/// Request to update an entity's display name.
#[derive(Clone, Debug)]
pub struct RenameEntityRequest {
    /// Entity to rename.
    pub entity: EntityRef,
    /// New user-visible display name.
    pub display_name: String,
}

/// Request to move an entity into or out of a project.
#[derive(Clone, Debug)]
pub struct MoveEntityRequest {
    /// Entity to move.
    pub entity: EntityRef,
    /// Destination project id, or `None` to move the entity to the root.
    pub project_id: Option<String>,
}

/// Request to duplicate an entity.
#[derive(Clone, Debug)]
pub struct DuplicateEntityRequest {
    /// Source entity to duplicate.
    pub entity: EntityRef,
    /// Optional display name for the new entity.
    pub display_name: Option<String>,
}

/// Request to update an entity's public and channel share policy.
#[derive(Clone, Debug)]
pub struct UpdateEntitySharePolicyRequest {
    /// Entity whose share policy should change.
    pub entity: EntityRef,
    /// Shared permission update used by documents, projects, chats, email
    /// threads, and calls.
    pub policy: UpdateSharePermissionRequestV2,
}

/// Stable machine-readable mutation failure category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityMutationErrorCode {
    /// The operation does not apply to this entity kind.
    UnsupportedOperation,
    /// The request is syntactically valid but violates a domain constraint.
    InvalidInput,
    /// The actor is authenticated but lacks the required capability.
    Forbidden,
    /// The referenced entity does not exist.
    NotFound,
    /// The requested mutation conflicts with current entity state.
    Conflict,
    /// The mutation failed for an internal reason.
    Internal,
}

/// Safe error returned for one item in a batch mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityMutationError {
    /// Machine-readable failure category.
    pub code: EntityMutationErrorCode,
    /// User-safe explanation of the failure.
    pub message: String,
}

impl EntityMutationError {
    /// Construct a mutation error.
    pub fn new(code: EntityMutationErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Log an internal failure and return the generic user-safe error.
    ///
    /// Callers run inside a per-item tracing span carrying the operation and
    /// entity fields, so the log line stays attributable.
    pub fn internal(detail: &dyn std::fmt::Debug) -> Self {
        tracing::error!(error = ?detail, "unified entity mutation failed");
        Self::new(EntityMutationErrorCode::Internal, "entity mutation failed")
    }

    /// Construct a not-found error.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(EntityMutationErrorCode::NotFound, message)
    }

    /// Construct a forbidden error.
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(EntityMutationErrorCode::Forbidden, message)
    }

    /// Construct an invalid-input error.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(EntityMutationErrorCode::InvalidInput, message)
    }

    /// Construct a state-conflict error.
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(EntityMutationErrorCode::Conflict, message)
    }
}

/// Result for one requested entity in a batch mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityMutationOutcome {
    /// Entity reference supplied by the caller.
    pub requested: EntityRef,
    /// Entity produced or updated by the operation. Duplicate operations use
    /// this field for the newly created entity.
    pub entity: Option<EntityRef>,
    /// Canonical records known to have changed as a consequence of the
    /// request. This includes affected containers and includes cascade
    /// descendants when the delegated domain service exposes their ids.
    pub affected_entities: Vec<EntityRef>,
    /// Per-item failure. A missing error denotes success.
    pub error: Option<EntityMutationError>,
}

impl EntityMutationOutcome {
    /// Build a successful outcome that changed only the requested entity.
    pub fn success(requested: EntityRef) -> Self {
        Self {
            entity: Some(requested.clone()),
            affected_entities: vec![requested.clone()],
            requested,
            error: None,
        }
    }

    /// Build a successful outcome with an explicit result and affected set.
    pub fn success_with(
        requested: EntityRef,
        entity: Option<EntityRef>,
        affected_entities: Vec<EntityRef>,
    ) -> Self {
        Self {
            requested,
            entity,
            affected_entities,
            error: None,
        }
    }

    /// Build a failed outcome.
    pub fn failure(requested: EntityRef, error: EntityMutationError) -> Self {
        Self {
            requested,
            entity: None,
            affected_entities: Vec::new(),
            error: Some(error),
        }
    }

    /// Build a standard unsupported-operation outcome.
    pub fn unsupported(requested: EntityRef, operation: &str) -> Self {
        let kind = requested.entity_type.to_string();
        Self::failure(
            requested,
            EntityMutationError::new(
                EntityMutationErrorCode::UnsupportedOperation,
                format!("{operation} is not supported for {kind} entities"),
            ),
        )
    }
}
