//! Ordered choice lists for the configuration enums the wizards present as
//! radio groups.
//!
//! Both interfaces used to encode each enum four times over: the order and
//! labels when building the list, and the inverse mapping when applying the
//! selection — once per interface. Nothing tied the four together, so adding a
//! variant meant four correct edits with no compiler help, and the two
//! interfaces had already drifted apart on what an out-of-range index means.
//!
//! The order and the labels live here, beside the enums. A test pins the
//! labels to each enum's `Display`, so the text the wizard shows and the text
//! the enum prints cannot drift apart. An index is now only ever produced and
//! consumed through this trait.

use super::types::{
    AudioServer, CompressionAlgo, InitSystem, InstallationMode, SeatAccess, SwapMode,
    ZfsEncryptionMode,
};

/// A configuration value offered as an ordered list of alternatives.
///
/// `CHOICES` is the single source of truth for what the list contains and what
/// position each value occupies. Implementations must list every variant; the
/// `choice_roundtrip` test helper checks that for each one.
pub trait Choice: Copy + PartialEq + Sized + 'static {
    /// The alternatives in presentation order, each with the text shown for
    /// it. Borrowed rather than owned so both interfaces can use these
    /// directly: the terminal one holds `&'static str` in its menu rows.
    const CHOICES: &'static [(Self, &'static str)];

    /// Position in the presented list.
    ///
    /// A value missing from `ORDER` is a bug in the implementation rather than
    /// something a caller can act on, so this falls back to the first entry
    /// instead of forcing every call site into error handling. The round-trip
    /// tests exist to keep that from happening.
    fn index(self) -> usize {
        Self::CHOICES
            .iter()
            .position(|(value, _)| *value == self)
            .unwrap_or(0)
    }

    /// The alternative at `index`, or `None` if the index is out of range.
    fn from_index(index: usize) -> Option<Self> {
        Self::CHOICES.get(index).map(|(value, _)| *value)
    }

    /// The text shown for this alternative.
    fn label(self) -> &'static str {
        Self::CHOICES
            .iter()
            .find(|(value, _)| *value == self)
            .map(|(_, label)| *label)
            .unwrap_or_default()
    }

    /// Labels for the whole list, in order.
    fn labels() -> Vec<&'static str> {
        Self::CHOICES.iter().map(|(_, label)| *label).collect()
    }
}

impl Choice for InstallationMode {
    const CHOICES: &'static [(Self, &'static str)] = &[
        (Self::FullDisk, "Full Disk"),
        (Self::NewPool, "New Pool"),
        (Self::ExistingPool, "Existing Pool"),
    ];
}

impl Choice for CompressionAlgo {
    // `off` last: it is the opt-out, and the interfaces render the final row
    // of this group as the "disabled" state.
    const CHOICES: &'static [(Self, &'static str)] = &[
        (Self::Lz4, "lz4"),
        (Self::Zstd, "zstd"),
        (Self::Zstd5, "zstd-5"),
        (Self::Zstd10, "zstd-10"),
        (Self::Off, "off"),
    ];
}

impl Choice for ZfsEncryptionMode {
    const CHOICES: &'static [(Self, &'static str)] = &[
        (Self::None, "No encryption"),
        (Self::Pool, "Encrypt entire pool"),
        (Self::Dataset, "Encrypt base dataset only"),
    ];
}

impl Choice for SwapMode {
    const CHOICES: &'static [(Self, &'static str)] = &[
        (Self::None, "None"),
        (Self::Zram, "ZRAM"),
        (Self::ZswapPartition, "Swap partition"),
        (Self::ZswapPartitionEncrypted, "Swap partition (encrypted)"),
    ];
}

impl Choice for InitSystem {
    const CHOICES: &'static [(Self, &'static str)] =
        &[(Self::Dracut, "dracut"), (Self::Mkinitcpio, "mkinitcpio")];
}

/// Optional settings are presented with an explicit "None" row first, so the
/// `Option` itself is the choice rather than something the interfaces have to
/// offset their indices around.
impl Choice for Option<AudioServer> {
    const CHOICES: &'static [(Self, &'static str)] = &[
        (None, "None"),
        (Some(AudioServer::Pipewire), "pipewire"),
        (Some(AudioServer::Pulseaudio), "pulseaudio"),
    ];
}

impl Choice for Option<SeatAccess> {
    const CHOICES: &'static [(Self, &'static str)] = &[
        (None, "None"),
        (Some(SeatAccess::Seatd), "seatd"),
        (Some(SeatAccess::Polkit), "polkit"),
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every listed choice maps to its own index and back.
    fn roundtrip<T: Choice + std::fmt::Debug>() {
        for (position, (choice, _)) in T::CHOICES.iter().enumerate() {
            assert_eq!(choice.index(), position, "index of {choice:?}");
            assert_eq!(
                T::from_index(position),
                Some(*choice),
                "value at index {position}"
            );
        }
        assert_eq!(T::labels().len(), T::CHOICES.len());
        assert!(T::from_index(T::CHOICES.len()).is_none(), "out of range");
    }

    #[test]
    fn every_choice_list_round_trips() {
        roundtrip::<InstallationMode>();
        roundtrip::<CompressionAlgo>();
        roundtrip::<ZfsEncryptionMode>();
        roundtrip::<SwapMode>();
        roundtrip::<InitSystem>();
        roundtrip::<Option<AudioServer>>();
        roundtrip::<Option<SeatAccess>>();
    }

    /// Exhaustiveness: a new variant must be added to `CHOICES`, and these
    /// matches stop compiling until it is listed here too.
    #[test]
    fn every_variant_is_listed() {
        fn assert_listed<T: Choice + std::fmt::Debug>(variants: &[T]) {
            for variant in variants {
                assert!(
                    T::CHOICES.iter().any(|(value, _)| value == variant),
                    "{variant:?} is missing from CHOICES, so no interface can offer it"
                );
            }
        }

        assert_listed(&[
            InstallationMode::FullDisk,
            InstallationMode::NewPool,
            InstallationMode::ExistingPool,
        ]);
        assert_listed(&[
            CompressionAlgo::Off,
            CompressionAlgo::Lz4,
            CompressionAlgo::Zstd,
            CompressionAlgo::Zstd5,
            CompressionAlgo::Zstd10,
        ]);
        assert_listed(&[
            ZfsEncryptionMode::None,
            ZfsEncryptionMode::Pool,
            ZfsEncryptionMode::Dataset,
        ]);
        assert_listed(&[
            SwapMode::None,
            SwapMode::Zram,
            SwapMode::ZswapPartition,
            SwapMode::ZswapPartitionEncrypted,
        ]);
        assert_listed(&[InitSystem::Dracut, InitSystem::Mkinitcpio]);
        assert_listed(&[
            None,
            Some(AudioServer::Pipewire),
            Some(AudioServer::Pulseaudio),
        ]);
        assert_listed(&[None, Some(SeatAccess::Seatd), Some(SeatAccess::Polkit)]);
    }

    /// The wizard's labels are the enums' own `Display` output. Keeping them
    /// as literals in the table lets both interfaces borrow them, and this is
    /// what stops the two representations drifting apart.
    #[test]
    fn labels_match_the_display_impls() {
        fn assert_display_matches<T: Choice + std::fmt::Display + std::fmt::Debug>() {
            for (value, label) in T::CHOICES {
                assert_eq!(
                    &value.to_string(),
                    label,
                    "{value:?} prints differently from the label the wizard shows"
                );
            }
        }

        assert_display_matches::<InstallationMode>();
        assert_display_matches::<CompressionAlgo>();
        assert_display_matches::<ZfsEncryptionMode>();
        assert_display_matches::<SwapMode>();
        assert_display_matches::<InitSystem>();

        // Option has no Display of its own; check the inner values and the
        // spelling of the empty row.
        for (value, label) in <Option<AudioServer>>::CHOICES {
            match value {
                Some(server) => assert_eq!(&server.to_string(), label),
                None => assert_eq!(*label, "None"),
            }
        }
        for (value, label) in <Option<SeatAccess>>::CHOICES {
            match value {
                Some(access) => assert_eq!(&access.to_string(), label),
                None => assert_eq!(*label, "None"),
            }
        }
    }
}
