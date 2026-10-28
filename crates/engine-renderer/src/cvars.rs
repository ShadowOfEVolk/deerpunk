//! Renderer cvars (`r_*`, `video.cfg` / `RENDERINFO`).

use deerpunk_engine_common::cvar_def::RENDERINFO;

deerpunk_engine_common::cvar_def!(
    "r_showfps",
    "0",
    "Alias of cl_showfps (legacy).",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_speeds",
    "0",
    "Render HUD (0=off, 1=GoldSrc speeds block, 2=+CPU encode, 3=+draw counts, 4=+shader diag).",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_showalloc",
    "0",
    "ImGui mesh GPU pool bars (0=off, 1=all allocators, N>1=only allocator id N-1).",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_fullbright",
    "0",
    "Ignore lightmaps (1=albedo only, HL-style fullbright).",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_vsync",
    "1",
    "Vertical sync (1=FIFO/vsync on, 0=uncapped when GPU supports Immediate).",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_lightmap",
    "0",
    "Debug: draw lightmap only (1=on).",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_loggpu",
    "0",
    "Log GPU resource uploads (0=off, 1=when uploads occur, 2=every frame).",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_phys_debug",
    "0",
    "Wireframe world collider part under crosshair (0=off, 1=on).",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_drawentities",
    "0",
    "Draw client entities (0=off, 1=bbox, 2=eye lines, 3=both, 4=mesh AABB).",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_exposure",
    "1",
    "Exposure for tonemap. -1 = auto-exposure, otherwise fixed multiplier. 1 = GoldSrc-style.",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_tonemap",
    "-1",
    "Tonemap mode: -1=passthrough (GoldSrc-style raw), 0=ACES, 1=Reinhard, 2=Hable.",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_bloom",
    "1",
    "Enable HDR bloom postfx (0/1).",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_bloom_threshold",
    "0.7",
    "Bloom luminance threshold.",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_bloom_intensity",
    "1",
    "Bloom add intensity.",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_bloom_downscale",
    "2",
    "Bloom buffer scale divisor (2=half res, 4=quarter — faster, softer).",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_grain",
    "1",
    "Enable film grain postfx (0/1).",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_grain_strength",
    "0.025",
    "Film grain strength.",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_frustum_cull",
    "1",
    "Cull mesh draws outside the camera frustum (0=off)",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_cull_cell_size",
    "32",
    "Spatial grid cell size for frustum broad-phase (world units)",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_occlusion_cull",
    "0",
    "GPU occlusion cull mesh draws (AABB queries, 1 frame latency; 0=off)",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_grain_downscale",
    "3",
    "Film grain noise tile scale.",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_cloud_speed",
    "0.04",
    "Procedural cloud wind speed (shader time scale).",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_cloud_mode",
    "0",
    "Cloud pass: 0=soft volumetric blend, 1=cheap Bayer dither (no soft alpha).",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_sky_turbidity",
    "3.0",
    "Hosek-Wilkie atmosphere turbidity 1..10 (higher = hazier).",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_sky_albedo",
    "0.25",
    "Hosek-Wilkie ground albedo 0..1 (single value applied to RGB).",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "r_sky_debug",
    "0",
    "Log Hosek SkyEnvironment each scene extract (1=on).",
    RENDERINFO
);
