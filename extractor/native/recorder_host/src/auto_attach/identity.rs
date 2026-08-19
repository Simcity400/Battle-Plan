#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ProcessFiletime(pub u64);

impl ProcessFiletime {
    pub fn unix_seconds(self) -> i64 {
        const UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;
        if self.0 < UNIX_EPOCH_TICKS {
            return i64::MIN;
        }
        ((self.0 - UNIX_EPOCH_TICKS) / 10_000_000) as i64
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ProcessKey {
    pub pid: u32,
    pub creation_filetime: ProcessFiletime,
}

impl ProcessKey {
    pub fn new(pid: u32, creation_filetime: u64) -> Self {
        Self {
            pid,
            creation_filetime: ProcessFiletime(creation_filetime),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub key: ProcessKey,
    pub image_path: String,
    pub session_id: u32,
    pub user_sid_hash: String,
}

impl ProcessIdentity {
    pub fn image_basename_lower(&self) -> String {
        let norm = self.image_path.replace('\\', "/");
        norm.rsplit('/')
            .next()
            .unwrap_or(&norm)
            .to_ascii_lowercase()
    }

    pub fn is_hoi4(&self) -> bool {
        self.image_basename_lower() == BATTLE_PLAN_EXE_LOWER
    }
}

pub const BATTLE_PLAN_EXE_LOWER: &str = "hoi4.exe";

pub fn name_is_hoi4(name_or_path: &str) -> bool {
    let norm = name_or_path.replace('\\', "/");
    let base = norm.rsplit('/').next().unwrap_or(&norm);
    base.eq_ignore_ascii_case(BATTLE_PLAN_EXE_LOWER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filetime_full_precision_not_floored_to_seconds() {
        const BASE: u64 = 133_000_000_000_000_000;
        let a = ProcessKey::new(9, BASE);
        let b = ProcessKey::new(9, BASE + 1);
        assert_ne!(a, b);
        assert_eq!(
            a.creation_filetime.unix_seconds(),
            b.creation_filetime.unix_seconds(),
            "they DO share the same whole second — proving second-precision would collide them"
        );
    }

    #[test]
    fn hoi4_name_match_is_case_and_separator_insensitive() {
        assert!(name_is_hoi4("hoi4.exe"));
        assert!(name_is_hoi4("HOI4.EXE"));
        assert!(name_is_hoi4("Hoi4.Exe"));
        assert!(name_is_hoi4(
            r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe"
        ));
        assert!(name_is_hoi4("C:/Games/hoi4.exe"));
        assert!(!name_is_hoi4("hoi4_launcher.exe"));
        assert!(!name_is_hoi4("notepad.exe"));
        assert!(!name_is_hoi4("dowser.exe"));
    }

    #[test]
    fn identity_basename_and_is_hoi4() {
        let id = ProcessIdentity {
            key: ProcessKey::new(10, 500),
            image_path: r"D:\Steam\common\the game\Hoi4.exe".to_string(),
            session_id: 1,
            user_sid_hash: "abc".to_string(),
        };
        assert_eq!(id.image_basename_lower(), "hoi4.exe");
        assert!(id.is_hoi4());
    }
}
