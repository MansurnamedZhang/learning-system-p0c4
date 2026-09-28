use uuid::Uuid;
/// Construct only after authentication, never from an untrusted request body.
#[derive(Debug, Clone, Copy)]
pub struct Principal {
    pub actor_id: Uuid,
}
