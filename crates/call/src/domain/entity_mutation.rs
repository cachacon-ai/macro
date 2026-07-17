//! Unified entity-mutation capability impls for calls.

use entity_access::domain::models::{
    AccessError, EditAccessLevel, EntityAccessReceipt, ViewAccessLevel,
};
use entity_mutation::{
    DeleteEntityPermanently, EntityMutationError, EntityRef, RenameEntity, UpdateEntitySharePolicy,
};
use models_permissions::share_permission::UpdateSharePermissionRequestV2;

use super::{
    models::{CallError, EditCallRecordRequest},
    ports::CallService,
    service::CallServiceImpl,
};
use connection::domain::ports::ConnectionService;
use notification::domain::{ports::VoipPushSender, service::NotificationIngress};

use crate::domain::ports::{
    CallRepository, CallRtcClient, CallSearchIndexer, CallSummarizer, RecordingStorage,
    VoiceRepository,
};

impl From<CallError> for EntityMutationError {
    fn from(error: CallError) -> Self {
        match error {
            CallError::NotFound(_) => Self::not_found("call not found"),
            CallError::Auth | CallError::NotInCall => {
                Self::forbidden("insufficient call permission")
            }
            CallError::InvalidRequest(message) => Self::invalid(message),
            CallError::AlreadyInCall(_) => Self::conflict(error.to_string()),
            error @ CallError::Internal(_) => Self::internal(&error),
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

/// Reject the mutation while the call is still active.
async fn require_archived_call<S: CallService>(
    service: &S,
    edit_receipt: &EntityAccessReceipt<EditAccessLevel>,
    operation: &str,
) -> Result<(), EntityMutationError> {
    let view_receipt = edit_receipt
        .clone()
        .try_into_requirement::<ViewAccessLevel>()
        .map_err(access_error)?;
    if service.get_call_record(view_receipt).await?.is_active {
        return Err(EntityMutationError::conflict(format!(
            "cannot {operation} an active call"
        )));
    }
    Ok(())
}

impl<R, C, Cn, E, N, S, Sm, I, V, Vr> RenameEntity
    for CallServiceImpl<R, C, Cn, E, N, S, Sm, I, V, Vr>
where
    R: CallRepository,
    C: CallRtcClient,
    Cn: ConnectionService,
    E: entity_access::domain::ports::EntityAccessService,
    N: NotificationIngress,
    S: RecordingStorage,
    Sm: CallSummarizer,
    I: CallSearchIndexer,
    V: VoipPushSender,
    Vr: VoiceRepository,
    Self: CallService,
{
    type Receipt = EditAccessLevel;

    async fn rename_entity(
        &self,
        _entity: EntityRef,
        receipt: EntityAccessReceipt<Self::Receipt>,
        display_name: String,
    ) -> Result<Vec<EntityRef>, EntityMutationError> {
        require_archived_call(self, &receipt, "rename").await?;
        self.edit_call_record(
            receipt,
            EditCallRecordRequest {
                share_permission: None,
                share_with_team: None,
                custom_name: Some(display_name),
            },
        )
        .await?;
        Ok(Vec::new())
    }
}

impl<R, C, Cn, E, N, S, Sm, I, V, Vr> UpdateEntitySharePolicy
    for CallServiceImpl<R, C, Cn, E, N, S, Sm, I, V, Vr>
where
    R: CallRepository,
    C: CallRtcClient,
    Cn: ConnectionService,
    E: entity_access::domain::ports::EntityAccessService,
    N: NotificationIngress,
    S: RecordingStorage,
    Sm: CallSummarizer,
    I: CallSearchIndexer,
    V: VoipPushSender,
    Vr: VoiceRepository,
    Self: CallService,
{
    type Receipt = EditAccessLevel;

    async fn update_share_policy(
        &self,
        _entity: EntityRef,
        receipt: EntityAccessReceipt<Self::Receipt>,
        policy: UpdateSharePermissionRequestV2,
    ) -> Result<Vec<EntityRef>, EntityMutationError> {
        self.edit_call_record(
            receipt,
            EditCallRecordRequest {
                share_permission: Some(policy),
                share_with_team: None,
                custom_name: None,
            },
        )
        .await?;
        Ok(Vec::new())
    }
}

impl<R, C, Cn, E, N, S, Sm, I, V, Vr> DeleteEntityPermanently
    for CallServiceImpl<R, C, Cn, E, N, S, Sm, I, V, Vr>
where
    R: CallRepository,
    C: CallRtcClient,
    Cn: ConnectionService,
    E: entity_access::domain::ports::EntityAccessService,
    N: NotificationIngress,
    S: RecordingStorage,
    Sm: CallSummarizer,
    I: CallSearchIndexer,
    V: VoipPushSender,
    Vr: VoiceRepository,
    Self: CallService,
{
    type Receipt = EditAccessLevel;

    async fn delete_entity_permanently(
        &self,
        _entity: EntityRef,
        receipt: EntityAccessReceipt<Self::Receipt>,
    ) -> Result<Vec<EntityRef>, EntityMutationError> {
        require_archived_call(self, &receipt, "permanently delete").await?;
        self.delete_call_record(receipt).await?;
        Ok(Vec::new())
    }
}
