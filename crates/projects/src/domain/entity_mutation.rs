//! Unified entity-mutation capability impls for projects.

use entity_access::domain::models::{
    AccessError, EditAccessLevel, EntityAccessReceipt, OwnerAccessLevel,
};
use entity_access_management::domain::ports::EntityAccessManagementService;
use entity_mutation::{
    DeleteEntityPermanently, EntityMutationError, EntityRef, MoveEntity, RenameEntity,
    RestoreEntity, TrashEntity, UpdateEntitySharePolicy, capability::project_refs,
};
use macro_event_broker::MacroEventBroker;
use model::project::{BasicProject, request::PatchProjectRequestV2};
use model_entity::EntityType;
use models_permissions::share_permission::UpdateSharePermissionRequestV2;

use super::{
    models::ProjectError,
    ports::{
        BulkUploadRequestPort, ProjectRepo, ProjectSearchIndexer, ProjectService,
        ProjectUploadUrlPort, ShaCounterPort,
    },
    service::ProjectServiceImpl,
};

impl From<ProjectError> for EntityMutationError {
    fn from(error: ProjectError) -> Self {
        match error {
            ProjectError::NotFound(_) => Self::not_found("project not found"),
            ProjectError::Unauthorized => Self::forbidden("insufficient project permission"),
            ProjectError::UnauthorizedWithMessage(message) => Self::forbidden(message),
            ProjectError::BadRequest(message) => Self::invalid(message),
            ProjectError::NameTooLong { max } => {
                Self::invalid(format!("display name exceeds {max} characters"))
            }
            ProjectError::CannotModifyDeleted => Self::conflict("cannot modify a deleted project"),
            ProjectError::RecursiveNesting => Self::invalid("project move would create a cycle"),
            error @ ProjectError::Internal(_) => Self::internal(&error),
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

/// Fetch project metadata; access is enforced separately via receipts.
async fn basic_project<S: ProjectService>(
    service: &S,
    entity: &EntityRef,
) -> Result<BasicProject, EntityMutationError> {
    Ok(service
        .internal_get_basic_project(&entity.entity_id)
        .await?)
}

impl<R, U, D, Sha, Eam, Idx, B> RenameEntity for ProjectServiceImpl<R, U, D, Sha, Eam, Idx, B>
where
    R: ProjectRepo,
    U: ProjectUploadUrlPort,
    D: BulkUploadRequestPort,
    Sha: ShaCounterPort,
    Eam: EntityAccessManagementService,
    Idx: ProjectSearchIndexer,
    B: MacroEventBroker,
    Self: ProjectService,
{
    type Receipt = EditAccessLevel;

    async fn rename_entity(
        &self,
        entity: EntityRef,
        receipt: EntityAccessReceipt<Self::Receipt>,
        display_name: String,
    ) -> Result<Vec<EntityRef>, EntityMutationError> {
        let project = basic_project(self, &entity).await?;
        let parent_id = project.parent_id.clone();
        self.edit_project(
            receipt,
            project,
            PatchProjectRequestV2 {
                name: Some(display_name),
                project_parent_id: None,
                share_permission: None,
            },
        )
        .await?;
        Ok(project_refs([parent_id]))
    }
}

impl<R, U, D, Sha, Eam, Idx, B> MoveEntity for ProjectServiceImpl<R, U, D, Sha, Eam, Idx, B>
where
    R: ProjectRepo,
    U: ProjectUploadUrlPort,
    D: BulkUploadRequestPort,
    Sha: ShaCounterPort,
    Eam: EntityAccessManagementService,
    Idx: ProjectSearchIndexer,
    B: MacroEventBroker,
    Self: ProjectService,
{
    type Receipt = OwnerAccessLevel;

    async fn move_entity(
        &self,
        entity: EntityRef,
        receipt: EntityAccessReceipt<Self::Receipt>,
        project_id: Option<String>,
        _project_receipt: Option<EntityAccessReceipt<EditAccessLevel>>,
    ) -> Result<Vec<EntityRef>, EntityMutationError> {
        let project = basic_project(self, &entity).await?;
        let old_parent_id = project.parent_id.clone();
        // Moving requires ownership at the API surface; the edit call itself
        // consumes an edit-level receipt. Self-parenting, cycles, and
        // deleted-state are enforced by the project domain service; an empty
        // parent id means "root".
        let receipt = receipt
            .try_into_requirement::<EditAccessLevel>()
            .map_err(access_error)?;
        self.edit_project(
            receipt,
            project,
            PatchProjectRequestV2 {
                name: None,
                project_parent_id: Some(project_id.clone().unwrap_or_default()),
                share_permission: None,
            },
        )
        .await?;
        Ok(project_refs([old_parent_id, project_id]))
    }
}

impl<R, U, D, Sha, Eam, Idx, B> UpdateEntitySharePolicy
    for ProjectServiceImpl<R, U, D, Sha, Eam, Idx, B>
where
    R: ProjectRepo,
    U: ProjectUploadUrlPort,
    D: BulkUploadRequestPort,
    Sha: ShaCounterPort,
    Eam: EntityAccessManagementService,
    Idx: ProjectSearchIndexer,
    B: MacroEventBroker,
    Self: ProjectService,
{
    type Receipt = OwnerAccessLevel;

    async fn update_share_policy(
        &self,
        entity: EntityRef,
        receipt: EntityAccessReceipt<Self::Receipt>,
        policy: UpdateSharePermissionRequestV2,
    ) -> Result<Vec<EntityRef>, EntityMutationError> {
        let project = basic_project(self, &entity).await?;
        let receipt = receipt
            .try_into_requirement::<EditAccessLevel>()
            .map_err(access_error)?;
        self.edit_project(
            receipt,
            project,
            PatchProjectRequestV2 {
                name: None,
                project_parent_id: None,
                share_permission: Some(policy),
            },
        )
        .await?;
        Ok(Vec::new())
    }
}

impl<R, U, D, Sha, Eam, Idx, B> TrashEntity for ProjectServiceImpl<R, U, D, Sha, Eam, Idx, B>
where
    R: ProjectRepo,
    U: ProjectUploadUrlPort,
    D: BulkUploadRequestPort,
    Sha: ShaCounterPort,
    Eam: EntityAccessManagementService,
    Idx: ProjectSearchIndexer,
    B: MacroEventBroker,
    Self: ProjectService,
{
    type Receipt = OwnerAccessLevel;

    async fn trash_entity(
        &self,
        entity: EntityRef,
        receipt: EntityAccessReceipt<Self::Receipt>,
    ) -> Result<Vec<EntityRef>, EntityMutationError> {
        let project = basic_project(self, &entity).await?;
        let actor = receipt
            .get_authenticated_user()
            .map_err(access_error)?
            .to_string();
        let deleted = self.soft_delete_project(receipt, project, actor).await?;
        Ok(refs(EntityType::Project, deleted.project_ids)
            .chain(refs(EntityType::Document, deleted.document_ids))
            .chain(refs(EntityType::Chat, deleted.chat_ids))
            .collect())
    }
}

impl<R, U, D, Sha, Eam, Idx, B> RestoreEntity for ProjectServiceImpl<R, U, D, Sha, Eam, Idx, B>
where
    R: ProjectRepo,
    U: ProjectUploadUrlPort,
    D: BulkUploadRequestPort,
    Sha: ShaCounterPort,
    Eam: EntityAccessManagementService,
    Idx: ProjectSearchIndexer,
    B: MacroEventBroker,
    Self: ProjectService,
{
    type Receipt = OwnerAccessLevel;

    async fn restore_entity(
        &self,
        entity: EntityRef,
        receipt: EntityAccessReceipt<Self::Receipt>,
    ) -> Result<Vec<EntityRef>, EntityMutationError> {
        let project = basic_project(self, &entity).await?;
        let parent_id = project.parent_id.clone();
        self.revert_delete_project(receipt, project).await?;
        Ok(project_refs([parent_id]))
    }
}

impl<R, U, D, Sha, Eam, Idx, B> DeleteEntityPermanently
    for ProjectServiceImpl<R, U, D, Sha, Eam, Idx, B>
where
    R: ProjectRepo,
    U: ProjectUploadUrlPort,
    D: BulkUploadRequestPort,
    Sha: ShaCounterPort,
    Eam: EntityAccessManagementService,
    Idx: ProjectSearchIndexer,
    B: MacroEventBroker,
    Self: ProjectService,
{
    type Receipt = OwnerAccessLevel;

    async fn delete_entity_permanently(
        &self,
        entity: EntityRef,
        receipt: EntityAccessReceipt<Self::Receipt>,
    ) -> Result<Vec<EntityRef>, EntityMutationError> {
        let project = basic_project(self, &entity).await?;
        let parent_id = project.parent_id.clone();
        self.permanently_delete_project(receipt, project).await?;
        Ok(project_refs([parent_id]))
    }
}

/// Build entity refs of one kind from raw ids.
fn refs(
    entity_type: EntityType,
    ids: impl IntoIterator<Item = String>,
) -> impl Iterator<Item = EntityRef> {
    ids.into_iter()
        .map(move |id| EntityRef::new(entity_type, id))
}
