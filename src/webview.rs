/// Advanced is a more complex interface than basic and assumes the user stores all the view ids themselves.
/// This gives the user more freedom by allowing them to view multiple views at the same time, but removes
/// actions like close current
pub mod advanced;
/// Basic allows users to have simple interfaces like close current and
/// allows users to index views by ints like 0, 1 , or 2
pub mod basic;

/// Shader-based rendering widget for engines that manage their own scrolling
/// (e.g. servo). Uses direct GPU texture updates to avoid Handle cache churn.
#[cfg(feature = "servo")]
pub(crate) mod shader_widget;
