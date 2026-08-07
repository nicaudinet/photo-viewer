//! One module per screen. Each owns its own state, actions and view; `App`
//! (in `app/`) owns the screen-independent state and the transitions between
//! them.

pub mod empty;
pub mod single;
pub mod wall;

pub(crate) const ICON_MARGIN: f32 = 10.0;
