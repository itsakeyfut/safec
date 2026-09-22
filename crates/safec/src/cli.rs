//! Command line interface.
//!
//! Short flags belong to the C compiler convention. `safec` claims one only
//! where it means exactly what `cc` means by it, so that an existing build can
//! eventually set `CC=safec` without `-c`, `-S`, `-I`, `-D` and friends
//! changing meaning underneath it. Everything this project invents gets a long
//! flag of its own.

use std::path::{Path, PathBuf};

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};

use safec_ir::target::Target;

use crate::options::{ColorMode, EmitKind, Options};
use crate::safety::SafetyLevel;

/// The triple this compiler was built to run on.
///
/// Set by `build.rs` from cargo's `TARGET`, which is the only thing that knows
/// it exactly. It is what `--target` defaults to.
pub const HOST_TRIPLE: &str = env!("SAFEC_HOST_TRIPLE");

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

    /// Report `Unknown` analysis results as warnings rather than errors.
    ///
    /// Wherever a safety check runs, a conclusion it could not prove fails the
    /// build, because that is what asking to be checked means here. This is the
    /// way back to a warning for a program that is still being migrated. See
    /// ADR-0033.
    ///
    /// Does nothing at `--safety off`, where no check runs and there is no
    /// unproven conclusion to report, and is refused beside `--safety strict`,
    /// which is defined as leaving nothing `Unknown`: see [`Cli::check`].
    ///
    /// The forerunner of a lint level system. When `--warn <LINT>` arrives,
    /// `unknown` becomes its first lint and this spelling becomes a hidden
    /// alias rather than a second way of saying the same thing.
    #[arg(long)]
    pub allow_unknown: bool,

    /// The machine to compile for.
    ///
    /// Validated by clap against the triples [`Target::ALL`] holds, so an
    /// unknown one is reported with the known ones listed, the way an unknown
    /// `--emit` is. Defaults to [`HOST_TRIPLE`]; ADR-0013 says why the host
    /// picking the default is not the host reaching the output.
    #[arg(
        long,
        value_name = "TRIPLE",
        default_value = HOST_TRIPLE,
        value_parser = clap::builder::PossibleValuesParser::new(
            Target::ALL.iter().map(|target| target.triple()).collect::<Vec<_>>()
        )
    )]
    pub target: String,

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
    /// Refuse an invocation that asks for two different things.
    ///
    /// `--safety strict` is defined as leaving nothing `Unknown`, so
    /// `--allow-unknown` beside it cannot be honoured. [`Policy::new`] answers
    /// that pair by denying, because a policy has to be total for a caller that
    /// never saw a command line, and this is what stops the user finding that
    /// out from a build's worth of errors rather than from one line.
    ///
    /// Separate from clap's own `conflicts_with`, which relates two arguments
    /// and cannot be told about one *value* of `--safety`. Reported as a
    /// [`clap::Error`] all the same, so that it prints and exits the way every
    /// other argument mistake does: nothing has been read at this point, so
    /// there is no source text to put a caret into.
    ///
    /// [`Policy::new`]: crate::diagnostics::Policy::new
    pub fn check(&self) -> Result<(), clap::Error> {
        if self.allow_unknown && self.safety >= SafetyLevel::Strict {
            return Err(Self::as_invoked().error(
                ErrorKind::ArgumentConflict,
                "--allow-unknown cannot be used with --safety strict, \
                 which is defined as leaving nothing unknown",
            ));
        }

        Ok(())
    }

    /// The command, named the way this process was invoked.
    ///
    /// `clap` fills its `Usage:` line from a bin name it takes off `argv[0]`
    /// while parsing. A [`clap::Command`] built afterwards never saw `argv[0]`
    /// and falls back to the `name` in the derive above, so a refusal reported
    /// through one tells a user who ran `cc` to go and re-read the usage of
    /// `safec`. The rename this module's own doc comment is about is exactly
    /// the case where that is wrong.
    fn as_invoked() -> clap::Command {
        let command = Self::command();

        match std::env::args_os().next() {
            // The file name rather than the path, because that is what `clap`
            // prints for the errors it raises itself. A second spelling here
            // would make one refusal disagree with every other.
            Some(argv0) => match Path::new(&argv0).file_name() {
                Some(name) => command.bin_name(name.to_string_lossy().into_owned()),
                None => command,
            },
            None => command,
        }
    }

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
            target,
            allow_unknown,
            color,
        } = self;

        Options {
            inputs,
            output,
            safety,
            emit,
            // clap answered for the spelling against `Target::ALL`, so the only
            // way here is a triple that table holds.
            target: Target::from_triple(&target).expect("clap accepted this triple"),
            // Carried across unresolved. This used to resolve the strictest
            // level here as well, so that `Options::deny_unknown` read
            // truthfully to anyone who inspected it; the field is now the
            // request rather than an answer, and `Policy::new` is the one place
            // that turns the pair into what the sink does. See ADR-0004.
            allow_unknown,
            color,
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::ValueEnum;

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
        assert!(!cli.allow_unknown);
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
    fn allow_unknown_is_set_by_its_flag() {
        let cli = Cli::try_parse_from(["safec", "--allow-unknown", "a.c"]).unwrap();
        assert!(cli.allow_unknown);
    }

    /// The request crosses the boundary unresolved, at every level.
    ///
    /// This replaces `a_lower_safety_level_leaves_deny_unknown_to_its_flag`,
    /// which asserted what ADR-0033 reverses. `Cli::into_options` used to
    /// resolve the strictest level here as well, so that `Options` read
    /// truthfully to anyone who inspected it; the field is now what the user
    /// asked for and `Policy::new` is the only place that turns the pair into
    /// what the sink does, so what is left for this layer to hold is that it
    /// does not decide early. `every_level_that_runs_a_check_denies_unknown_`
    /// `unless_it_was_allowed` in `diagnostics.rs` is where the rule itself is.
    ///
    /// Mutation: resolve it here, `allow_unknown: allow_unknown && safety <
    /// SafetyLevel::Strict`. The strict row fails, naming the level.
    #[test]
    fn every_safety_level_carries_the_request_across_the_boundary_unchanged() {
        for level in ["off", "memory", "lifetime", "ownership", "thread", "strict"] {
            for asked in [false, true] {
                let mut args = vec!["safec", "--safety", level];
                if asked {
                    args.push("--allow-unknown");
                }
                args.push("a.c");

                let options = Cli::try_parse_from(args).unwrap().into_options();

                assert_eq!(
                    options.allow_unknown, asked,
                    "--safety {level} did not carry allow_unknown = {asked}"
                );
            }
        }
    }

    /// The level defined as leaving nothing `Unknown` cannot also be asked to
    /// leave some.
    ///
    /// [`Policy::new`] answers the pair by denying, so honouring the flag is
    /// not what is at stake: being told in one line, before a file is read, is.
    ///
    /// Mutation: drop the `self.safety >= SafetyLevel::Strict` conjunct, so
    /// every run that allows unknown is refused. The test below fails on its
    /// second loop. The other conjunct is held by that test's first loop, and
    /// only since this was measured: without the `strict` row there, dropping
    /// `self.allow_unknown &&` refuses every strict run with the whole suite
    /// green.
    ///
    /// [`Policy::new`]: crate::diagnostics::Policy::new
    #[test]
    fn allowing_unknown_is_refused_at_the_level_defined_as_leaving_none() {
        let cli = Cli::try_parse_from(["safec", "--safety", "strict", "--allow-unknown", "a.c"])
            .expect("the arguments parse; the conflict is not clap's to see");

        assert_eq!(
            cli.check().expect_err("the pair is refused").kind(),
            ErrorKind::ArgumentConflict
        );
    }

    /// Only the pair is refused, and neither half on its own is.
    ///
    /// Both loops are load-bearing and each holds one conjunct of the refusal.
    /// The first covers every level with nothing asked, `strict` included,
    /// which is the row that was missing when this was written: without it, a
    /// `check` that refuses every strict run leaves the whole suite green.
    /// Measured, which is how the row came to be here.
    #[test]
    fn an_invocation_that_asks_for_only_one_of_the_two_is_accepted() {
        for level in ["off", "memory", "lifetime", "ownership", "thread", "strict"] {
            let cli = Cli::try_parse_from(["safec", "--safety", level, "a.c"]).unwrap();

            assert!(cli.check().is_ok(), "--safety {level} alone was refused");
        }

        for level in ["off", "memory", "lifetime", "ownership", "thread"] {
            let cli = Cli::try_parse_from(["safec", "--safety", level, "--allow-unknown", "a.c"])
                .unwrap();

            assert!(
                cli.check().is_ok(),
                "--safety {level} with the flag was refused"
            );
        }
    }

    /// Every argument has to survive the trip across the CLI boundary. The two
    /// halves are guarded by the compiler rather than by this test: a field
    /// added to `Options` breaks the struct literal below, and one added to
    /// `Cli` breaks the destructuring in `into_options`. What is left for the
    /// test is that each argument arrives as the value that was parsed.
    ///
    /// `--safety thread` rather than `strict`, because `--allow-unknown` beside
    /// the strictest level is an invocation [`Cli::check`] refuses, and pinning
    /// the trip made by one nobody can run is pinning nothing.
    #[test]
    fn every_argument_survives_the_trip_into_options() {
        let options = Cli::try_parse_from([
            "safec",
            "--safety",
            "thread",
            "--emit",
            "safety-ir",
            "--allow-unknown",
            "--color",
            "never",
            "--target",
            "wasm32-unknown-unknown",
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
                safety: SafetyLevel::Thread,
                emit: EmitKind::SafetyIr,
                // Named in the arguments above, and a triple nothing hosts.
                // Leaving `--target` out and writing a host's triple here made
                // this pass on the machine it was written on and fail on the
                // other two CI runners, which is the defect
                // `a_target_the_host_is_not` exists to catch in the corpus and
                // the same one, one layer up.
                target: Target::from_triple("wasm32-unknown-unknown").expect("a known triple"),
                allow_unknown: true,
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

    /// A triple nothing measured is refused, with the known ones listed.
    ///
    /// The same shape as an unknown `--emit`, and for the same reason: this
    /// compiler cannot say what `char` is worth on a machine nobody measured,
    /// and guessing is how an artifact starts lying about its target.
    ///
    /// Mutation: take the triple as a plain `String` with no `value_parser`.
    /// The parse succeeds and this fails.
    #[test]
    fn rejects_a_triple_this_compiler_does_not_know() {
        assert_eq!(
            parse_error(&["safec", "--target", "x86_64-unknown-none", "a.c"]),
            ErrorKind::InvalidValue
        );
    }

    /// `--target` defaults to the machine this compiler was built to run on,
    /// and naming one overrides it.
    ///
    /// The host picking the default is not the host reaching the output:
    /// ADR-0013 records the difference, and the corpus is what holds it, by
    /// naming a target and comparing bytes on three runners whose hosts differ.
    ///
    /// Mutation: default to a fixed triple rather than to `HOST_TRIPLE`. The
    /// first assertion fails on every host but that one. Mutation: ignore
    /// `--target`. The second fails.
    #[test]
    fn the_target_defaults_to_the_host_and_a_flag_overrides_it() {
        let defaulted = Cli::try_parse_from(["safec", "a.c"])
            .expect("a bare invocation parses")
            .into_options();
        assert_eq!(defaulted.target.triple(), HOST_TRIPLE);

        let named = Cli::try_parse_from(["safec", "--target", "wasm32-unknown-unknown", "a.c"])
            .expect("a known triple parses")
            .into_options();
        assert_eq!(named.target.triple(), "wasm32-unknown-unknown");
    }

    /// The machine this was built for is one this compiler knows.
    ///
    /// `--target` defaults to `HOST_TRIPLE`, and clap validates it against
    /// `Target::ALL`, so a host missing from that table makes every bare
    /// invocation fail. Better to find that here than in somebody's terminal.
    ///
    /// Mutation: remove this host's row from `Target::ALL`. This fails, and so
    /// does the test above.
    #[test]
    fn the_host_this_was_built_for_is_a_target_this_compiler_knows() {
        assert!(
            Target::from_triple(HOST_TRIPLE).is_some(),
            "{HOST_TRIPLE} is not in `Target::ALL`: measure it with              `clang --target={HOST_TRIPLE} -dM -E -x c /dev/null` and add the row"
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
