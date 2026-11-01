use clap::{Parser, ValueEnum};

#[derive(Parser, Debug, Clone)]
#[command(
    name = "lyricli",
    version,
    about = "Real-time synced Spotify and MPRIS lyrics in the terminal with ASCII cover art"
)]
pub struct Cli {
    /// Specific MPRIS player name (defaults to prioritizing 'spotify', then active players)
    #[arg(short, long)]
    pub player: Option<String>,

    /// Hide the ASCII album cover art (can also be toggled live with 'c')
    #[arg(long)]
    pub no_cover: bool,

    /// Character style for the ASCII album cover (can also be cycled live with 'v')
    #[arg(short = 'c', long, value_enum, default_value_t = CoverStyle::Braille)]
    pub charset: CoverStyle,

    /// ASCII typography style for the active lyric line (can also be cycled live with 'f')
    #[arg(short = 'l', long, value_enum, default_value_t = LyricArtStyle::Block)]
    pub lyric_style: LyricArtStyle,

    /// Hide surrounding plain-text lyric lines so only the ASCII lyric is shown (toggle live with 't')
    #[arg(long)]
    pub hide_text: bool,

    /// Show surrounding plain-text lyric lines above and below the ASCII lyric (toggle live with 't')
    #[arg(long)]
    pub show_text: bool,

    /// Width of the ASCII album cover in terminal columns
    #[arg(long, default_value_t = 34)]
    pub cover_width: u32,

    /// Hide the bottom keybind bar on startup (can also be toggled live with 'h')
    #[arg(long, visible_alias = "no-hud")]
    pub hide_controls: bool,

    /// Initial lyric timing offset in milliseconds (negative = earlier, positive = later)
    #[arg(short, long, default_value_t = 0)]
    pub offset: i64,

    /// Search and display lyrics for a specific track title without an active player
    #[arg(long)]
    pub track: Option<String>,

    /// Artist name when using --track
    #[arg(long)]
    pub artist: Option<String>,

    /// Print the lyrics of the current or specified track to stdout and exit
    #[arg(long)]
    pub print: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum CoverStyle {
    Braille,
    Blocks,
    Detailed,
    Standard,
}

impl CoverStyle {
    pub fn next(self) -> Self {
        match self {
            Self::Braille => Self::Blocks,
            Self::Blocks => Self::Detailed,
            Self::Detailed => Self::Standard,
            Self::Standard => Self::Braille,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Braille => "braille",
            Self::Blocks => "blocks",
            Self::Detailed => "detailed",
            Self::Standard => "standard",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum LyricArtStyle {
    Block,
    Braille,
    Banner,
}

impl LyricArtStyle {
    pub fn next(self) -> Self {
        match self {
            Self::Block => Self::Braille,
            Self::Braille => Self::Banner,
            Self::Banner => Self::Block,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::Braille => "braille",
            Self::Banner => "banner",
        }
    }
}

