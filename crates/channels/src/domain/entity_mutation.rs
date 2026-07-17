//! Unified entity-mutation capability impls for channels.

use entity_access::domain::models::{
    AdminParticipantRole, EntityAccessReceipt, OwnerParticipantRole, RequiredPermission,
};
use entity_mutation::{DeleteEntityPermanently, EntityMutationError, EntityRef, RenameEntity};
use uuid::Uuid;

use super::{
    models::{PatchChannelRequest, Sender},
    ports::{ChannelMutationErr, ChannelService},
    service::ChannelServiceImpl,
};

impl From<ChannelMutationErr> for EntityMutationError {
    fn from(error: ChannelMutationErr) -> Self {
        match error {
            ChannelMutationErr::BadRequest(message) => Self::invalid(message),
            ChannelMutationErr::Unauthorized(_) => Self::forbidden("insufficient channel role"),
            ChannelMutationErr::NotFound(_) => Self::not_found("channel not found"),
            error => Self::internal(&error),
        }
    }
}

/// Parse the channel id, rejecting non-UUID identifiers.
fn channel_uuid(entity: &EntityRef) -> Result<Uuid, EntityMutationError> {
    Uuid::parse_str(&entity.entity_id)
        .map_err(|_| EntityMutationError::invalid("entity id must be a UUID"))
}

/// Convert the authenticated receipt holder into a channel sender.
fn sender_from_receipt<T: RequiredPermission>(
    receipt: &EntityAccessReceipt<T>,
) -> Result<Sender, EntityMutationError> {
    receipt
        .get_authenticated_user()
        .cloned()
        .map(Sender::new_from_user)
        .map_err(|_| EntityMutationError::forbidden("authenticated user required"))
}

impl<R, E, P, M> RenameEntity for ChannelServiceImpl<R, E, P, M>
where
    Self: ChannelService,
{
    type Receipt = AdminParticipantRole;

    async fn rename_entity(
        &self,
        entity: EntityRef,
        receipt: EntityAccessReceipt<Self::Receipt>,
        display_name: String,
    ) -> Result<Vec<EntityRef>, EntityMutationError> {
        let channel_id = channel_uuid(&entity)?;
        let sender = sender_from_receipt(&receipt)?;
        self.patch_channel(
            sender,
            channel_id,
            PatchChannelRequest {
                channel_name: Some(display_name),
                convert_to_team_channel: None,
                auto_join_team: None,
            },
        )
        .await?;
        Ok(Vec::new())
    }
}

impl<R, E, P, M> DeleteEntityPermanently for ChannelServiceImpl<R, E, P, M>
where
    Self: ChannelService,
{
    type Receipt = OwnerParticipantRole;

    async fn delete_entity_permanently(
        &self,
        entity: EntityRef,
        receipt: EntityAccessReceipt<Self::Receipt>,
    ) -> Result<Vec<EntityRef>, EntityMutationError> {
        let channel_id = channel_uuid(&entity)?;
        let sender = sender_from_receipt(&receipt)?;
        self.delete_channel(sender, channel_id).await?;
        Ok(Vec::new())
    }
}
