//! Command line interface.
//!
//! Short flags belong to the C compiler convention. `safec` claims one only
//! where it means exactly what `cc` means by it, so that an existing build can
//! eventually set `CC=safec` without `-c`, `-S`, `-I`, `-D` and friends
//! changing meaning underneath it. Everything this project invents gets a long
//! flag of its own.

use std::path::PathBuf;

use clap::Parser;

use crate::options::{ColorMode, EmitKind, Options};
use crate::safety::SafetyLevel;

/// A small, hackable, safety-oriented C compiler.
#[derive(Debug, Parser)]
#[command(name = "safec", version, about, long_about = None)]
pub struct Cli {
    /// C source files to compile.
    #[arg(value_name = "FILE", required = true)]
    pub inputs: Vec<PathBuf>,

    /// Write the result to this path.
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<PathBuf>,

    /// How much of the safety model to enforce.
    #[arg(
        long,
        value_enum,
        value_name = "LEVEL",
        default_value_t = SafetyLevel::Memory
    )]
    pub safety: SafetyLevel,

    /// The artifact to produce.
    #[arg(
        long,
        value_enum,
        value_name = "KIND",
        default_value_t = EmitKind::Executable
    )]
    pub emit: EmitKind,

    /// Report `Unknown` analysis results as errors rather than warnings.
    ///
    /// Implied by `--safety strict`, which is defined as leaving nothing
    /// `Unknown`. [`Cli::into_options`] resolves the two into one value.
    ///
    /// The forerunner of a lint level system. When `--deny <LINT>` arrives,
    /// `unknown` becomes its first lint and this spelling becomes a hidden
    /// alias rather than a second way of saying the same thing.
    #[arg(long)]
    pub deny_unknown: bool,

    /// When to colorize diagnostics.
    #[arg(
        long,
        value_enum,
        value_name = "WHEN",
        default_value_t = ColorMode::Auto
    )]
    pub color: ColorMode,
}

impl Cli {
    /// Resolve the arguments into the options the compiler runs on.
    ///
    /// The mapping is one to one today. It exists so that the compiler depends
    /// on [`Options`] rather than on a clap type, and it is where defaults that
    /// have to be computed, such as an output path derived from the inputs,
    /// will be worked out.
    pub fn into_options(self) -> Options {
        // Destructured rather than read field by field, so that a field added
        // to `Cli` and forgotten here is a compile error instead of an argument
        // that silently does nothing.
        let Cli {
            inputs,
            output,
            safety,
            emit,
            deny_unknown,
            color,
        } = self;

        Options {
            inputs,
            output,
            safety,
            emit,
            // `--safety strict` is defined as leaving nothing `Unknown`, so it
            // carries `--deny-unknown` with it. Resolved once here rather than
            // in every consumer of `Options`, which is what this method is for.
            deny_unknown: deny_unknown || safety == SafetyLevel::Strict,
            color,
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;
    use clap::ValueEnum;
    use clap::error::ErrorKind;

    use super::*;

    /// The kind of error clap reports for an invocation that must not parse.
    ///
    /// `try_parse_from` also fails for `--help` and `--version`, so a bare
    /// `is_err()` would accept a failure for entirely the wrong reason.
    fn parse_error(args: &[&str]) -> ErrorKind {
        Cli::try_parse_from(args)
            .expect_err("the parse was expected to fail")
            .kind()
    }

    /// The spellings a value enum exposes on the command line, in declaration
    /// order.
    fn value_names<T: ValueEnum>() -> Vec<String> {
        T::value_variants()
            .iter()
            .map(|variant| variant.to_possible_value().unwrap().get_name().to_owned())
            .collect()
    }

    #[test]
    fn command_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn safety_levels_are_spelled_in_kebab_case() {
        assert_eq!(
            value_names::<SafetyLevel>(),
            ["off", "memory", "lifetime", "ownership", "thread", "strict"]
        );
    }

    #[test]
    fn emit_kinds_are_spelled_in_kebab_case() {
        assert_eq!(
            value_names::<EmitKind>(),
            [
                "tokens",
                "ast",
                "safety-ir",
                "llvm-ir",
                "object",
                "executable"
            ]
        );
    }

    #[test]
    fn color_modes_are_spelled_in_kebab_case() {
        assert_eq!(value_names::<ColorMode>(), ["auto", "always", "never"]);
    }

    #[test]
    fn every_safety_level_round_trips_through_the_parser() {
        for level in SafetyLevel::value_variants() {
            let name = level.to_possible_value().unwrap();
            let name = name.get_name();
            let cli = Cli::try_parse_from(["safec", "--safety", name, "a.c"]).unwrap();
            assert_eq!(cli.safety, *level, "--safety {name} did not round trip");
        }
    }

    #[test]
    fn every_emit_kind_round_trips_through_the_parser() {
        for kind in EmitKind::value_variants() {
            let name = kind.to_possible_value().unwrap();
            let name = name.get_name();
            let cli = Cli::try_parse_from(["safec", "--emit", name, "a.c"]).unwrap();
            assert_eq!(cli.emit, *kind, "--emit {name} did not round trip");
        }
    }

    #[test]
    fn every_color_mode_round_trips_through_the_parser() {
        for mode in ColorMode::value_variants() {
            let name = mode.to_possible_value().unwrap();
            let name = name.get_name();
            let cli = Cli::try_parse_from(["safec", "--color", name, "a.c"]).unwrap();
            assert_eq!(cli.color, *mode, "--color {name} did not round trip");
        }
    }

    /// The artifact is spelled out, but cc habits die hard.
    #[test]
    fn accepts_exe_as_an_alias_for_the_executable() {
        let cli = Cli::try_parse_from(["safec", "--emit", "exe", "a.c"]).unwrap();
        assert_eq!(cli.emit, EmitKind::Executable);
    }

    #[test]
    fn a_bare_invocation_uses_the_documented_defaults() {
        let cli = Cli::try_parse_from(["safec", "main.c"]).unwrap();
        assert_eq!(cli.safety, SafetyLevel::Memory);
        assert_eq!(cli.emit, EmitKind::Executable);
        assert_eq!(cli.color, ColorMode::Auto);
        assert_eq!(cli.output, None);
        assert!(!cli.deny_unknown);
    }

    #[test]
    fn keeps_every_input_file_in_the_order_given() {
        let cli = Cli::try_parse_from(["safec", "a.c", "b.c", "c.c"]).unwrap();
        assert_eq!(
            cli.inputs,
            [
                PathBuf::from("a.c"),
                PathBuf::from("b.c"),
                PathBuf::from("c.c"),
            ]
        );
    }

    #[test]
    fn accepts_an_explicit_safety_level_before_the_inputs() {
        let cli = Cli::try_parse_from(["safec", "--safety", "lifetime", "a.c", "b.c"]).unwrap();
        assert_eq!(cli.safety, SafetyLevel::Lifetime);
        assert_eq!(cli.inputs, [PathBuf::from("a.c"), PathBuf::from("b.c")]);
    }

    #[test]
    fn accepts_an_output_path_through_either_spelling_of_the_flag() {
        for flag in ["-o", "--output"] {
            let cli = Cli::try_parse_from(["safec", flag, "out.bin", "a.c"]).unwrap();
            assert_eq!(
                cli.output,
                Some(PathBuf::from("out.bin")),
                "{flag} was ignored"
            );
        }
    }

    #[test]
    fn deny_unknown_is_set_by_its_flag() {
        let cli = Cli::try_parse_from(["safec", "--deny-unknown", "a.c"]).unwrap();
        assert!(cli.deny_unknown);
    }

    /// The strictest level is defined as leaving nothing `Unknown`, so asking
    /// for it is asking for `--deny-unknown`. Resolving that here rather than
    /// in each consumer is what keeps the two flags from disagreeing.
    #[test]
    fn the_strictest_safety_level_denies_unknown_on_its_own() {
        let options = Cli::try_parse_from(["safec", "--safety", "strict", "a.c"])
            .unwrap()
            .into_options();

        assert!(options.deny_unknown);
    }

    #[test]
    fn a_lower_safety_level_leaves_deny_unknown_to_its_flag() {
        for level in ["off", "memory", "lifetime", "ownership", "thread"] {
            let options = Cli::try_parse_from(["safec", "--safety", level, "a.c"])
                .unwrap()
                .into_options();

            assert!(!options.deny_unknown, "--safety {level} denied unknown");
        }
    }

    /// Every argument has to survive the trip across the CLI boundary. The two
    /// halves are guarded by the compiler rather than by this test: a field
    /// added to `Options` breaks the struct literal below, and one added to
    /// `Cli` breaks the destructuring in `into_options`. What is left for the
    /// test is that each argument arrives as the value that was parsed.
    #[test]
    fn every_argument_survives_the_trip_into_options() {
        let options = Cli::try_parse_from([
            "safec",
            "--safety",
            "strict",
            "--emit",
            "safety-ir",
            "--deny-unknown",
            "--color",
            "never",
            "-o",
            "out.ir",
            "a.c",
            "b.c",
        ])
        .unwrap()
        .into_options();

        assert_eq!(
            options,
            Options {
                inputs: vec![PathBuf::from("a.c"), PathBuf::from("b.c")],
                output: Some(PathBuf::from("out.ir")),
                safety: SafetyLevel::Strict,
                emit: EmitKind::SafetyIr,
                deny_unknown: true,
                color: ColorMode::Never,
            }
        );
    }

    #[test]
    fn rejects_an_invocation_without_inputs() {
        assert_eq!(parse_error(&["safec"]), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn rejects_an_unknown_safety_level() {
        assert_eq!(
            parse_error(&["safec", "--safety", "paranoid", "a.c"]),
            ErrorKind::InvalidValue
        );
    }

    #[test]
    fn rejects_an_unknown_emit_kind() {
        assert_eq!(
            parse_error(&["safec", "--emit", "qbe", "a.c"]),
            ErrorKind::InvalidValue
        );
    }

    #[test]
    fn rejects_an_unknown_color_mode() {
        assert_eq!(
            parse_error(&["safec", "--color", "rainbow", "a.c"]),
            ErrorKind::InvalidValue
        );
    }

    /// Level names are matched case sensitively. Relaxing that is a deliberate
    /// decision, not something to slip in unnoticed.
    #[test]
    fn rejects_a_safety_level_in_the_wrong_case() {
        assert_eq!(
            parse_error(&["safec", "--safety", "MEMORY", "a.c"]),
            ErrorKind::InvalidValue
        );
    }

    #[test]
    fn rejects_an_unknown_flag() {
        assert_eq!(
            parse_error(&["safec", "a.c", "--bogus"]),
            ErrorKind::UnknownArgument
        );
    }

    #[test]
    fn rejects_an_output_flag_without_a_path() {
        assert_eq!(
            parse_error(&["safec", "a.c", "-o"]),
            ErrorKind::InvalidValue
        );
    }

    /// `--version` and `--help` are reported as errors as well, which is why
    /// every test above asserts on the error kind rather than on `is_err()`.
    #[test]
    fn version_and_help_are_reported_as_display_errors() {
        assert_eq!(
            parse_error(&["safec", "--version"]),
            ErrorKind::DisplayVersion
        );
        assert_eq!(parse_error(&["safec", "--help"]), ErrorKind::DisplayHelp);
    }
}
