//! Keep short vehicle animations available to capture while the renderer warms their pipelines.

use bevy::prelude::*;

pub fn wait_for_pipelines(
    pipelines: Res<aa_world::pipelines::PipelineStatus>,
    real: Res<Time<Real>>,
    mut time: ResMut<Time<Virtual>>,
    mut stalled_at: Local<Option<f64>>,
) {
    if pipelines.waiting() == 0 {
        time.unpause();
        *stalled_at = None;
    } else {
        time.pause();
        let started = *stalled_at.get_or_insert(real.elapsed_secs_f64());
        assert!(real.elapsed_secs_f64() - started < 30.0, "vehicle preview pipelines did not finish warming");
    }
}
