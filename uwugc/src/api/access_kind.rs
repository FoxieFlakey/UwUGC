use sealed::sealed;

// NOTE: This type is considered to be part of public API
#[sealed]
pub trait RefKind {}

// NOTE: This type is considered to be part of public API
pub struct Exclusive {}
#[sealed]
impl RefKind for Exclusive {}

// NOTE: This type is considered to be part of public API
pub struct Shared {}
#[sealed]
impl RefKind for Shared {}

