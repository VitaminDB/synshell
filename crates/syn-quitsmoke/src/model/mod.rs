//! Доменная модель QuitSmoke — чистые данные (serde), без зависимости от UI.

pub mod achievements;
pub mod health;
pub mod log;
pub mod profile;
pub mod settings;
pub mod stats;
pub mod taper;

pub use achievements::{AchKind, AchievementDef, AchievementState, ACHIEVEMENTS};
pub use health::{HealthMilestone, HEALTH_MILESTONES};
pub use log::{CigaretteLog, LogEntry};
pub use profile::{QuitMode, UserProfile};
pub use settings::{Locale, ReminderStyle, Settings};
pub use stats::DerivedStats;
pub use taper::{TaperCurve, TaperSchedule};
