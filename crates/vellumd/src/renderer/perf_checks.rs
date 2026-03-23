#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use vellum_ipc::ScaleMode;

    use crate::renderer::RendererState;

    #[test]
    fn apply_clear_stress_keeps_renderer_state_bounded() {
        let mut renderer = RendererState::default();
        renderer.refresh_outputs(vec!["DP-1".to_string(), "HDMI-A-1".to_string()]);

        for i in 0..250 {
            let target = if i % 2 == 0 {
                Some("DP-1".to_string())
            } else {
                None
            };
            renderer.enqueue_apply(
                target,
                PathBuf::from(format!("/tmp/wall-{i}.png")),
                ScaleMode::Fill,
            );
            if i % 5 == 0 {
                renderer.enqueue_clear();
            }
        }
        renderer
            .apply_pending()
            .expect("stress apply_pending should succeed");

        assert!(renderer.session_surface_count() <= 2);
        assert!(renderer.session_buffer_count() <= 8);
    }

    #[test]
    fn apply_pending_latency_check() {
        let mut renderer = RendererState::default();
        renderer.refresh_outputs(vec!["DP-1".to_string()]);

        for i in 0..400 {
            renderer.enqueue_apply(
                Some("DP-1".to_string()),
                PathBuf::from(format!("/tmp/latency-{i}.png")),
                ScaleMode::Crop,
            );
        }

        let started = Instant::now();
        renderer
            .apply_pending()
            .expect("latency apply_pending should succeed");
        let elapsed = started.elapsed();

        assert!(elapsed < Duration::from_secs(5));
    }
}
