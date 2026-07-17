//! Unified entity-mutation capability impls for chats.

use entity_access::domain::models::{
    AccessError, EditAccessLevel, EntityAccessReceipt, OwnerAccessLevel, ViewAccessLevel,
};
use entity_mutation::{
    DeleteEntityPermanently, DuplicateEntity, EntityMutationError, EntityRef, MoveEntity,
    RenameEntity, RestoreEntity, TrashEntity, UpdateEntitySharePolicy, capability::project_refs,
};
use macro_user_id::user_id::MacroUserIdStr;
use models_permissions::share_permission::UpdateSharePermissionRequestV2;

use crate::domain::{
    models::{ChatErr, PatchChatArgs},
    ports::ChatService,
};

use super::chat::ChatServiceImpl;

impl From<ChatErr> for EntityMutationError {
    fn from(error: ChatErr) -> Self {
        match error {
            ChatErr::NotFound => Self::not_found("chat not found"),
            ChatErr::BadRequest(message) => Self::invalid(message),
            ChatErr::Access(error) => access_error(error),
            error @ ChatErr::Unknown(_) => Self::internal(&error),
        }
    }
}

/// Map an access-domain error onto the public mutation vocabulary.
fn access_error(error: AccessError) -> EntityMutationError {
    match error {
        AccessError::Unauthorized | AccessError::UnauthorizedWithMessage(_) => {
            EntityMutationError::forbidden("insufficient permission for entity mutation")
        }
        AccessError::NotFound(_) => EntityMutationError::not_found("entity not found"),
        AccessError::BadRequest(message) => EntityMutationError::invalid(message),
        error @ (AccessError::DatabaseError(_) | AccessError::Internal) => {
            EntityMutationError::internal(&error)
        }
    }
}

/// Resolve the chat's containing project for affected-record reporting.
async fn chat_project_id<S: ChatService>(
    service: &S,
    owner_receipt: &EntityAccessReceipt<OwnerAccessLevel>,
) -> Result<Option<String>, EntityMutationError> {
    let view_receipt = owner_receipt
        .clone()
        .try_into_requirement::<ViewAccessLevel>()
        .map_err(access_error)?;
    Ok(service.get_metadata(view_receipt).await?.project_id)
}

impl<R, ToolSetContext, Eam> RenameEntity for ChatServiceImpl<R, ToolSetContext, Eam>
where
    ToolSetContext: Clone + Send + Sync + 'static,
    Self: ChatService,
{
    type Receipt = OwnerAccessLevel;

    async fn rename_entity(
        &self,
        _entity: EntityRef,
        receipt: EntityAccessReceipt<Self::Receipt>,
        display_name: String,
    ) -> Result<Vec<EntityRef>, EntityMutationError> {
        self.patch(
            receipt,
            PatchChatArgs {
                name: Some(display_name),
                project_id: None,
                share_permission: None,
            },
        )
        .await?;
        Ok(Vec::new())
    }
}

impl<R, ToolSetContext, Eam> MoveEntity for ChatServiceImpl<R, ToolSetContext, Eam>
where
    ToolSetContext: Clone + Send + Sync + 'static,
    Self: ChatService,
{
    type Receipt = OwnerAccessLevel;

    async fn move_entity(
        &self,
        _entity: EntityRef,
        receipt: EntityAccessReceipt<Self::Receipt>,
        project_id: Option<String>,
        _project_receipt: Option<EntityAccessReceipt<EditAccessLevel>>,
    ) -> Result<Vec<EntityRef>, EntityMutationError> {
        let old_project_id = chat_project_id(self, &receipt).await?;
        self.patch(
            receipt,
            PatchChatArgs {
                name: None,
                // The chat patch API uses an empty id to mean "root".
                project_id: Some(project_id.clone().unwrap_or_default()),
                share_permission: None,
            },
        )
        .await?;
        Ok(project_refs([old_project_id, project_id]))
    }
}

impl<R, ToolSetContext, Eam> UpdateEntitySharePolicy for ChatServiceImpl<R, ToolSetContext, Eam>
where
    ToolSetContext: Clone + Send + Sync + 'static,
    Self: ChatService,
{
    type Receipt = OwnerAccessLevel;

    async fn update_share_policy(
        &self,
        _entity: EntityRef,
        receipt: EntityAccessReceipt<Self::Receipt>,
        policy: UpdateSharePermissionRequestV2,
    ) -> Result<Vec<EntityRef>, EntityMutationError> {
        self.patch(
            receipt,
            PatchChatArgs {
                name: None,
                project_id: None,
                share_permission: Some(policy),
            },
        )
        .await?;
        Ok(Vec::new())
    }
}

impl<R, ToolSetContext, Eam> TrashEntity for ChatServiceImpl<R, ToolSetContext, Eam>
where
    ToolSetContext: Clone + Send + Sync + 'static,
    Self: ChatService,
{
    type Receipt = OwnerAccessLevel;

    async fn trash_entity(
        &self,
        _entity: EntityRef,
        receipt: EntityAccessReceipt<Self::Receipt>,
    ) -> Result<Vec<EntityRef>, EntityMutationError> {
        let project_id = chat_project_id(self, &receipt).await?;
        self.delete(receipt).await?;
        Ok(project_refs([project_id]))
    }
}

impl<R, ToolSetContext, Eam> RestoreEntity for ChatServiceImpl<R, ToolSetContext, Eam>
where
    ToolSetContext: Clone + Send + Sync + 'static,
    Self: ChatService,
{
    type Receipt = OwnerAccessLevel;

    async fn restore_entity(
        &self,
        _entity: EntityRef,
        receipt: EntityAccessReceipt<Self::Receipt>,
    ) -> Result<Vec<EntityRef>, EntityMutationError> {
        let project_id = chat_project_id(self, &receipt).await?;
        self.revert_delete(receipt).await?;
        Ok(project_refs([project_id]))
    }
}

impl<R, ToolSetContext, Eam> DeleteEntityPermanently for ChatServiceImpl<R, ToolSetContext, Eam>
where
    ToolSetContext: Clone + Send + Sync + 'static,
    Self: ChatService,
{
    type Receipt = OwnerAccessLevel;

    async fn delete_entity_permanently(
        &self,
        _entity: EntityRef,
        receipt: EntityAccessReceipt<Self::Receipt>,
    ) -> Result<Vec<EntityRef>, EntityMutationError> {
        let project_id = chat_project_id(self, &receipt).await?;
        self.permanently_delete(receipt).await?;
        Ok(project_refs([project_id]))
    }
}

impl<R, ToolSetContext, Eam> DuplicateEntity for ChatServiceImpl<R, ToolSetContext, Eam>
where
    ToolSetContext: Clone + Send + Sync + 'static,
    Self: ChatService,
{
    type Receipt = ViewAccessLevel;

    async fn duplicate_entity(
        &self,
        _entity: EntityRef,
        receipt: EntityAccessReceipt<Self::Receipt>,
        _user_id: MacroUserIdStr<'static>,
        display_name: Option<String>,
    ) -> Result<EntityRef, EntityMutationError> {
        if display_name.is_some() {
            return Err(EntityMutationError::invalid(
                "chat duplication does not yet accept a custom display name",
            ));
        }
        let id = self.copy_chat(receipt).await?;
        Ok(EntityRef::new(model_entity::EntityType::Chat, id))
    }
}
