use uuid::Uuid;
#[derive(Debug, thiserror::Error)]
pub enum ContentError {
    #[error("{0}")]
    Invalid(String),
    #[error("内容不存在或不可访问")]
    NotFound,
    #[error("基础修订已过期")]
    Conflict { current_revision_id: Uuid },
    #[error("幂等请求标识已用于其它内容")]
    IdempotencyConflict,
    #[error("存储服务暂时不可用")]
    Storage,
}
