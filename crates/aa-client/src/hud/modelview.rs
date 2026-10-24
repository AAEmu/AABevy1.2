//! Host-side character previews. Toolkit owns widget commands and image paint;
//! this module owns the original asset camera and the character's scene inputs.

mod camera;
mod fixture;
mod host;
mod readiness;
mod trial;
pub(crate) use host::ModelViewCamera;

pub(super) fn fixture() {
    fixture::run();
}

pub(super) fn plugin(app: &mut bevy::prelude::App) {
    readiness::plugin(app);
    host::plugin(app);
}
