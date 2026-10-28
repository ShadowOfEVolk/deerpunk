//! Client HUD / audio cvars (not `r_*` — those live in `deerpunk-engine-renderer`).

use deerpunk_engine_common::cvar_def::{ARCHIVE, RENDERINFO};

deerpunk_engine_common::cvar_def!(
    "cl_showfps",
    "0",
    "Show FPS in the client infobox (0=off, 1=on).",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "cl_showpos",
    "0",
    "Infobox pos/angles: 0=off, 1=pos+ang+view, 2=+speed, 3=+vel xyz+(speed).",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "net_show",
    "0",
    "Infobox net: 0=off, 1=role+rtt+B/s, 2=+loss+players.",
    RENDERINFO
);
deerpunk_engine_common::cvar_def!(
    "s_volume",
    "1",
    "Master audio volume (FMOD bus:/ volume, 0..1).",
    ARCHIVE
);
deerpunk_engine_common::cvar_def!("s_debug", "0", "Audio debug output (1=on).");
