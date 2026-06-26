use std::any::Any;
use std::sync::Arc;

/// Common runtime capability shared by service APIs that need active-thread or
/// active-turn context during one tool dispatch.
///
/// Domain-specific service API crates should depend on this trait rather than
/// baking concrete runtime types such as `TurnContext` into their public API.
pub trait ThreadCapability: Send + Sync + 'static {
    /// Return the concrete runtime object behind this capability.
    fn as_any(&self) -> &(dyn Any + Send + Sync);
}

impl<T> ThreadCapability for Arc<T>
where
    T: ThreadCapability,
{
    fn as_any(&self) -> &(dyn Any + Send + Sync) {
        self.as_ref().as_any()
    }
}
