//! Unified entity-mutation capability impls for email threads.

use entity_access::domain::models::{EditAccessLevel, EntityAccessReceipt};
use entity_mutation::{EntityMutationError, EntityRef, MoveEntity, capability::project_refs};

use super::EmailServiceImpl;
use crate::domain::{models::EmailErr, ports::EmailService};

impl From<EmailErr> for EntityMutationError {
    fn from(error: EmailErr) -> Self {
        match error {
            EmailErr::ThreadNotFound => Self::not_found("email thread not found"),
            EmailErr::Unauthorized => Self::forbidden("insufficient email thread permission"),
            error => Self::internal(&error),
        }
    }
}

impl<T, U, E, CS, Eam, B> MoveEntity for EmailServiceImpl<T, U, E, CS, Eam, B>
where
    Self: EmailService,
{
    type Receipt = EditAccessLevel;

    async fn move_entity(
        &self,
        _entity: EntityRef,
        receipt: EntityAccessReceipt<Self::Receipt>,
        project_id: Option<String>,
        project_receipt: Option<EntityAccessReceipt<EditAccessLevel>>,
    ) -> Result<Vec<EntityRef>, EntityMutationError> {
        let old_project_id = self.update_thread_project(receipt, project_receipt).await?;
        Ok(project_refs([old_project_id, project_id]))
    }
}
