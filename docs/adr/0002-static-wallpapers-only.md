# Static wallpapers only, no animated or video wallpapers

Vellum shows still images only (JPEG, PNG and WebP by default; AVIF and JXL as opt-in build features). Animated GIF/WebP/APNG and video are deliberately unsupported, even though 1.x had them. Animation needs per-frame decoding, frame caches and constant wakeups. That conflicts with our main goal: a Daemon that sleeps at zero CPU and holds almost no memory between wallpaper changes. Users who want motion should use a dedicated tool such as mpvpaper.
