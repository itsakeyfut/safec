//! Command line interface.
//!
//! Short flags belong to the C compiler convention. `safec` claims one only
//! where it means exactly what `cc` means by it, so that an existing build can
//! eventually set `CC=safec` without `-c`, `-S`, `-I`, `-D` and friends
//! changing meaning underneath it. Everything this project invents gets a long
//! flag of its own.

use std::path::{Path, PathBuf};

use clap::error::ErrorKind;
use clap::{CommandFactory, FromArgMatches, Parser};

use safec_ir::target::Target;

use crate::lexer::is_identifier;
use crate::options::{ColorMode, CommandLineMacro, EmitKind, Options};
use crate::safety::SafetyLevel;

/// The triple this compiler was built to run on.
///
/// Set by `build.rs` from cargo's `TARGET`, which is the only thing that knows
/// it exactly. It is what `--target` defaults to.
///
/// The `safec` package's tests read it here rather than through `env!`,
/// because a build script's variable reaches only its own package. See
/// ADR-0058.
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
    ///
    /// Defaults to what the artifact asked for can carry: `memory` where
    /// `--emit` reaches the Safety IR, and `off` for `tokens` and `ast`, which
    /// stop in the frontend and so run no check at all. Naming a level this run
    /// cannot deliver is reported rather than ignored. See ADR-0035.
    ///
    // `Option` rather than a clap default, so that resolving it is something
    // `Cli::into_options` cannot omit. A default printed here would be one
    // value, and one value is false for two of the six artifacts.
    //
    // A `//` comment rather than a `///` one: clap prints a doc comment into long
    // help verbatim, so this paragraph reached a user's terminal along with an
    // unrendered `[`Cli::into_options`]`. Nothing above it is addressed to a
    // maintainer, and nothing here is addressed to a user.
    #[arg(long, value_enum, value_name = "LEVEL")]
    pub safety: Option<SafetyLevel>,

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
    /// `--emit` is. Defaults to the machine this compiler was built for, and
    /// has to be named where that is a machine it does not know.
    ///
    // `Option` rather than a clap default, because clap validates a default as
    // if it had been typed: on a host missing from `Target::ALL` every run was
    // refused in clap's words, blaming a flag nobody gave. `resolved_target`
    // answers the default instead. ADR-0013 says why the host picking the
    // default is not the host reaching the output. `//` rather than `///`,
    // because clap prints a doc comment into `--help`.
    #[arg(
        long,
        value_name = "TRIPLE",
        value_parser = clap::builder::PossibleValuesParser::new(
            Target::ALL.iter().map(|target| target.triple()).collect::<Vec<_>>()
        )
    )]
    pub target: Option<String>,

    /// When to colorize diagnostics.
    #[arg(
        long,
        value_enum,
        value_name = "WHEN",
        default_value_t = ColorMode::Auto
    )]
    pub color: ColorMode,

    /// Define an object-like macro before the first line is read: `NAME`
    /// defines it as `1`.
    #[arg(short = 'D', value_name = "NAME[=VALUE]")]
    pub define: Vec<String>,

    /// Say that a macro is not defined when the first line is read.
    ///
    /// Also settles a name the compiler might predefine, such as `__GNUC__` or
    /// an include guard's `_FOO_H`, for `#ifdef` and `#ifndef`, which refuse
    /// such a name otherwise.
    // `-U` of a reserved name is a claim about the build that this compiler
    // cannot check, and the build analysed is the one it describes. See
    // ADR-0059. `//` rather than `///`, because clap prints a doc comment into
    // `--help`.
    #[arg(short = 'U', value_name = "NAME")]
    pub undefine: Vec<String>,

    // Which flag each `-D` and `-U` was, in the order they were given, which
    // the two fields above do not keep and `cc` applies them in. Filled by
    // `Cli::try_parse_in_order_from` from clap's indices, and not parsed.
    #[arg(skip)]
    order: Vec<MacroFlag>,
}

/// Which of the two flags a command-line macro was given with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MacroFlag {
    Define,
    Undefine,
}

impl Cli {
    /// Parse this process's arguments as [`Parser::parse`] does, and keep the
    /// order the `-D`s and `-U`s were given in.
    ///
    /// `cc` applies them left to right, so `-U X -D X=2` defines `X`, and that
    /// is the spelling a hardened build uses for `_FORTIFY_SOURCE`. The fields
    /// clap fills hold each flag on its own; its indices are what keep the
    /// order. See ADR-0059.
    pub fn parse_in_order() -> Self {
        Self::try_parse_in_order_from(std::env::args_os()).unwrap_or_else(|error| error.exit())
    }

    /// [`Self::parse_in_order`] of `args`, or the error clap reports.
    pub fn try_parse_in_order_from<I, T>(args: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        let matches = Self::command().try_get_matches_from(args)?;
        let mut cli =
            Self::from_arg_matches(&matches).map_err(|error| error.format(&mut Self::command()))?;
        let at = |id: &str, flag: MacroFlag| {
            matches
                .indices_of(id)
                .into_iter()
                .flatten()
                .map(move |index| (index, flag))
        };
        let mut order: Vec<(usize, MacroFlag)> = at("define", MacroFlag::Define)
            .chain(at("undefine", MacroFlag::Undefine))
            .collect();
        order.sort_by_key(|(index, _)| *index);
        cli.order = order.into_iter().map(|(_, flag)| flag).collect();
        Ok(cli)
    }

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
    ///
    /// Asked of the level the user *named*, which narrows nothing: an unnamed
    /// one resolves to `memory` or `off` in [`Self::into_options`] and neither
    /// reaches here. Still `>=` rather than `==`, because the ladder is
    /// cumulative and a level added above `strict` would owe this refusal too.
    pub fn check(&self) -> Result<(), clap::Error> {
        if self
            .safety
            .is_some_and(|level| level >= SafetyLevel::Strict)
            && self.allow_unknown
        {
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

    /// Resolve the arguments into the options the compiler runs on, or refuse
    /// a run that named no target on a machine this compiler does not know.
    ///
    /// Not a one to one mapping. It exists so that the compiler depends on
    /// [`Options`] rather than on a clap type, and it is where a default that
    /// has to be computed is worked out: `--safety` is one, because what a run
    /// can be held to depends on which artifact it asked for, and `--target`
    /// is another, because the host it defaults to may be no target at all.
    /// An output path derived from the inputs would be a third.
    ///
    /// **Fallible rather than checked first**, so that a caller cannot reach
    /// an `Options` without having answered the refusal: a precondition left
    /// to [`Self::check`] having run would be a panic wherever the order was
    /// broken. Reported as [`Self::check`] reports, through a [`clap::Error`]
    /// named the way this process was invoked, and with no code: it is about
    /// the machine, not a program, which `docs/diagnostics.md` rules out.
    ///
    /// The host is [`HOST_TRIPLE`]; `into_options_on` is the same with the
    /// host an argument, which is what lets a test ask about an unknown one.
    pub fn into_options(self) -> Result<Options, clap::Error> {
        self.into_options_on(HOST_TRIPLE)
    }

    /// [`Self::into_options`], on a named host.
    ///
    /// **The kind of the error is the exit code**, and that is why a test
    /// asks for it here rather than only for the message: `clap::Error::exit`
    /// writes `DisplayHelp` and `DisplayVersion` to stdout and exits 0, so the
    /// wrong kind is a refused run that reports success with nothing made.
    /// What no test can build is `main` exiting with the `Err` rather than
    /// discarding it: the `match` there is read rather than held, and a caller
    /// that takes the result as an [`Options`] is `error[E0308]`.
    fn into_options_on(self, host: &str) -> Result<Options, clap::Error> {
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
            define,
            undefine,
            order,
        } = self;

        let target = resolved_target(target.as_deref(), host).map_err(|message| {
            Self::as_invoked().error(ErrorKind::MissingRequiredArgument, message)
        })?;
        let macros = command_line_macros(&define, &undefine, &order)
            .map_err(|message| Self::as_invoked().error(ErrorKind::ValueValidation, message))?;

        Ok(Options {
            inputs,
            output,
            // Asked of the artifact rather than worked out here, so that a
            // caller with no command line behind it resolves this the same way:
            // the Clang adapter is that caller and ADR-0004 is the precedent.
            // `EmitKind::default_safety` carries why the default is not one
            // constant, and why it is `memory` rather than
            // `SafetyLevel::IMPLEMENTED`.
            safety: safety.unwrap_or(emit.default_safety()),
            emit,
            target,
            // Carried across unresolved. This used to resolve the strictest
            // level here as well, so that `Options::deny_unknown` read
            // truthfully to anyone who inspected it; the field is now the
            // request rather than an answer, and `Policy::new` is the one place
            // that turns the pair into what the sink does. See ADR-0004.
            allow_unknown,
            color,
            macros,
        })
    }
}

/// What `-D` and `-U` said, in the order `order` says they were given, or why
/// one cannot be read.
///
/// Refused here, before anything is read, so that it prints and exits as every
/// other argument mistake does:
///
/// - a name that is not an ASCII identifier. That covers an empty one and a
///   function-like `F(x)=x`, which this does not read, since only object-like
///   macros are read yet;
/// - a value holding a line break, which would split the line it becomes.
///
/// A `-D` is split at its first `=`, so `N=a=b` defines `N` as `a=b` and `N=`
/// defines it as nothing.
///
/// # Panics
///
/// If `order` does not name one flag for each `-D` and `-U`, which is a `Cli`
/// built some way other than [`Cli::try_parse_in_order_from`]. Taking every
/// `-D` first instead would describe `-U X -D X` as a build in which `X` is
/// undefined, which `cc` would not make.
fn command_line_macros(
    define: &[String],
    undefine: &[String],
    order: &[MacroFlag],
) -> Result<Vec<CommandLineMacro>, String> {
    assert_eq!(
        order.len(),
        define.len() + undefine.len(),
        "the order of `-D` and `-U` was not kept: parse with `Cli::try_parse_in_order_from`"
    );
    let (mut defines, mut undefines) = (define.iter(), undefine.iter());
    let mut macros = Vec::with_capacity(order.len());
    for flag in order {
        match flag {
            MacroFlag::Define => {
                let given = defines.next().expect("one `-D` for each in the order");
                let (name, value) = given.split_once('=').unwrap_or((given, "1"));
                if !is_identifier(name) {
                    return Err(format!(
                        "`-D {given}` is not read: `-D` takes an object-like macro, \
                         `NAME` or `NAME=VALUE`"
                    ));
                }
                if value.contains(['\n', '\r']) {
                    return Err(format!(
                        "`-D {name}` is not read: its value holds a line break"
                    ));
                }
                macros.push(CommandLineMacro::Defined {
                    name: name.to_owned(),
                    value: value.to_owned(),
                });
            }
            MacroFlag::Undefine => {
                let name = undefines.next().expect("one `-U` for each in the order");
                if !is_identifier(name) {
                    return Err(format!(
                        "`-U {name}` is not read: `-U` takes a macro's name"
                    ));
                }
                macros.push(CommandLineMacro::Undefined { name: name.clone() });
            }
        }
    }
    Ok(macros)
}

/// The target a run compiles for: the one it named, or the host's.
///
/// A function of both rather than of [`HOST_TRIPLE`], so that a test built on
/// a host this compiler knows can ask what one it does not know is told.
///
/// **An unknown host is refused, not defaulted.** Falling back to some row of
/// [`Target::ALL`] would compile for a machine nobody measured and say nothing,
/// and every byte of output would depend on that guess. The message lists the
/// rows in their order, so that a row added there is offered here without
/// anybody coming back.
fn resolved_target(named: Option<&str>, host: &str) -> Result<Target, String> {
    match named {
        // clap answered for the spelling against `Target::ALL`, so the only way
        // here is a triple that table holds.
        Some(named) => Ok(Target::from_triple(named).expect("clap accepted this triple")),
        None => Target::from_triple(host).ok_or_else(|| {
            let known: Vec<&str> = Target::ALL.iter().map(|target| target.triple()).collect();
            format!(
                "this compiler does not know the machine it was built for, `{host}`, \
                 so a run has to name one with `--target`: {}",
                known.join(", ")
            )
        }),
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
        Cli::try_parse_in_order_from(args)
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
                "hatches",
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
            let cli = Cli::try_parse_in_order_from(["safec", "--safety", name, "a.c"]).unwrap();
            assert_eq!(
                cli.safety,
                Some(*level),
                "--safety {name} did not round trip"
            );
        }
    }

    #[test]
    fn every_emit_kind_round_trips_through_the_parser() {
        for kind in EmitKind::value_variants() {
            let name = kind.to_possible_value().unwrap();
            let name = name.get_name();
            let cli = Cli::try_parse_in_order_from(["safec", "--emit", name, "a.c"]).unwrap();
            assert_eq!(cli.emit, *kind, "--emit {name} did not round trip");
        }
    }

    #[test]
    fn every_color_mode_round_trips_through_the_parser() {
        for mode in ColorMode::value_variants() {
            let name = mode.to_possible_value().unwrap();
            let name = name.get_name();
            let cli = Cli::try_parse_in_order_from(["safec", "--color", name, "a.c"]).unwrap();
            assert_eq!(cli.color, *mode, "--color {name} did not round trip");
        }
    }

    /// The artifact is spelled out, but cc habits die hard.
    #[test]
    fn accepts_exe_as_an_alias_for_the_executable() {
        let cli = Cli::try_parse_in_order_from(["safec", "--emit", "exe", "a.c"]).unwrap();
        assert_eq!(cli.emit, EmitKind::Executable);
    }

    /// `--safety` answers `None` rather than a level, because clap no longer
    /// supplies one: what a run defaults to depends on the artifact it asked
    /// for, and `the_default_level_is_what_the_artifact_can_carry` is where the
    /// resolved answer is held. Asserting a level here would put the default in
    /// two places.
    #[test]
    fn a_bare_invocation_uses_the_documented_defaults() {
        let cli = Cli::try_parse_in_order_from(["safec", "main.c"]).unwrap();
        assert_eq!(cli.safety, None);
        assert_eq!(cli.emit, EmitKind::Executable);
        assert_eq!(cli.color, ColorMode::Auto);
        assert_eq!(cli.output, None);
        assert!(!cli.allow_unknown);
    }

    /// What an unnamed `--safety` resolves to, for every artifact.
    ///
    /// Driven by the roster, because the answer is a property of the kind and a
    /// kind added later acquires one whether or not anybody wrote it down.
    ///
    /// Two assertions per row on purpose. The first is that clap supplies no
    /// level, which is what makes the second [`Cli::into_options`]'s answer
    /// rather than a default printed in two places; without it a clap default
    /// could be reintroduced and the second assertion would still hold.
    ///
    /// Mutation: resolve `None` to `SafetyLevel::Memory` unconditionally. The
    /// `tokens` and `ast` rows fail.
    ///
    /// **What this does not hold**: resolving to `SafetyLevel::IMPLEMENTED`
    /// instead of `Memory` fails nothing, because they are equal today. They are
    /// not the same thing, and the comment at the resolution says why. Nothing
    /// can hold that until a second level lands, which is the change that has to
    /// read it.
    #[test]
    fn the_default_level_is_what_the_artifact_can_carry() {
        for kind in EmitKind::value_variants() {
            let spelling = kind.spelling();
            let cli = Cli::try_parse_in_order_from(["safec", "--emit", spelling.as_str(), "a.c"])
                .expect("every kind is a value `--emit` takes");

            assert_eq!(cli.safety, None, "{kind:?}: clap supplied a level");
            assert_eq!(
                cli.into_options()
                    .expect("this host is a known target")
                    .safety,
                if kind.reaches_the_ir() {
                    SafetyLevel::Memory
                } else {
                    SafetyLevel::Off
                },
                "{kind:?}"
            );
        }
    }

    #[test]
    fn keeps_every_input_file_in_the_order_given() {
        let cli = Cli::try_parse_in_order_from(["safec", "a.c", "b.c", "c.c"]).unwrap();
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
        let cli =
            Cli::try_parse_in_order_from(["safec", "--safety", "lifetime", "a.c", "b.c"]).unwrap();
        assert_eq!(cli.safety, Some(SafetyLevel::Lifetime));
        assert_eq!(cli.inputs, [PathBuf::from("a.c"), PathBuf::from("b.c")]);
    }

    #[test]
    fn accepts_an_output_path_through_either_spelling_of_the_flag() {
        for flag in ["-o", "--output"] {
            let cli = Cli::try_parse_in_order_from(["safec", flag, "out.bin", "a.c"]).unwrap();
            assert_eq!(
                cli.output,
                Some(PathBuf::from("out.bin")),
                "{flag} was ignored"
            );
        }
    }

    #[test]
    fn allow_unknown_is_set_by_its_flag() {
        let cli = Cli::try_parse_in_order_from(["safec", "--allow-unknown", "a.c"]).unwrap();
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

                let options = Cli::try_parse_in_order_from(args)
                    .unwrap()
                    .into_options()
                    .expect("this host is a known target");

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
        let cli =
            Cli::try_parse_in_order_from(["safec", "--safety", "strict", "--allow-unknown", "a.c"])
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
            let cli = Cli::try_parse_in_order_from(["safec", "--safety", level, "a.c"]).unwrap();

            assert!(cli.check().is_ok(), "--safety {level} alone was refused");
        }

        for level in ["off", "memory", "lifetime", "ownership", "thread"] {
            let cli = Cli::try_parse_in_order_from([
                "safec",
                "--safety",
                level,
                "--allow-unknown",
                "a.c",
            ])
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
        let options = Cli::try_parse_in_order_from([
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
            "-DN=a=b",
            "-D",
            "M",
            "-UGUARD",
            "a.c",
            "b.c",
        ])
        .unwrap()
        .into_options()
        .expect("a named target needs no host");

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
                // Split at the first `=`, a bare name defined as `1`, and the
                // attached and separate spellings read alike.
                macros: vec![
                    CommandLineMacro::Defined {
                        name: "N".to_owned(),
                        value: "a=b".to_owned(),
                    },
                    CommandLineMacro::Defined {
                        name: "M".to_owned(),
                        value: "1".to_owned(),
                    },
                    CommandLineMacro::Undefined {
                        name: "GUARD".to_owned(),
                    },
                ],
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
        let defaulted = Cli::try_parse_in_order_from(["safec", "a.c"])
            .expect("a bare invocation parses")
            .into_options()
            .expect("this host is a known target");
        assert_eq!(defaulted.target.triple(), HOST_TRIPLE);

        let named =
            Cli::try_parse_in_order_from(["safec", "--target", "wasm32-unknown-unknown", "a.c"])
                .expect("a known triple parses")
                .into_options()
                .expect("a named target needs no host");
        assert_eq!(named.target.triple(), "wasm32-unknown-unknown");
    }

    /// The machine this was built for is one this compiler knows.
    ///
    /// `--target` defaults to `HOST_TRIPLE`, so on a host missing from
    /// `Target::ALL` every run that names no target is refused, by name and
    /// with the known triples listed. That is the right answer for a user and
    /// the wrong one for a contributor, who should measure the row instead,
    /// and this is what tells them to.
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

    /// A run that names no target on a machine this compiler does not know is
    /// refused, naming that machine and `--target`, and listing every triple
    /// it could name instead.
    ///
    /// `x86_64-unknown-linux-musl` is the host the defect was found on, by
    /// patching `build.rs` to report it. The test cannot be built on such a
    /// host, which is why the host is an argument.
    ///
    /// Mutation: have an unknown host fall back to `Target::ALL[0]`. This
    /// fails, and the run would have compiled for a machine nobody measured.
    /// Mutation: list a fixed set of triples rather than `Target::ALL`. This
    /// fails on the row the set leaves out.
    #[test]
    fn an_unknown_host_is_refused_by_name_and_told_every_target() {
        let message = resolved_target(None, "x86_64-unknown-linux-musl")
            .expect_err("an unknown host has no target to default to");

        assert!(message.contains("`x86_64-unknown-linux-musl`"), "{message}");
        assert!(message.contains("`--target`"), "{message}");
        // One row written out, so that the loop below is not the only thing
        // saying what the table holds.
        assert!(message.contains("x86_64-unknown-linux-gnu"), "{message}");
        for target in Target::ALL {
            assert!(message.contains(target.triple()), "{message}");
        }
    }

    /// The refusal of an unknown host fails the run: it is not help or a
    /// version, which clap prints to stdout and exits 0 for, and it goes to
    /// stderr with an exit code that says something went wrong.
    ///
    /// Mutation: refuse with `ErrorKind::DisplayVersion`. This fails, and the
    /// run would have printed its refusal to stdout and exited 0 having made
    /// nothing.
    #[test]
    fn an_unknown_host_fails_the_run_rather_than_reporting_success() {
        let refused = Cli::try_parse_in_order_from(["safec", "a.c"])
            .expect("a bare invocation parses")
            .into_options_on("x86_64-unknown-linux-musl")
            .expect_err("an unknown host has no target to default to");

        assert_eq!(refused.kind(), ErrorKind::MissingRequiredArgument);
        assert!(refused.use_stderr(), "the refusal goes to stdout");
        assert_ne!(refused.exit_code(), 0, "the refusal exits 0");
    }

    /// The refusal of a `-D` or `-U` this compiler cannot read, as a run
    /// would see it: its kind and its message.
    fn macro_refusal(args: &[&str]) -> (ErrorKind, String) {
        let refused = Cli::try_parse_in_order_from(args)
            .expect("the arguments parse")
            .into_options()
            .expect_err("the macro is refused");
        (refused.kind(), refused.to_string())
    }

    /// A `-D` that is not an object-like macro's name is refused before
    /// anything is read: a function-like one, an empty name, one beginning
    /// with a digit, and one that is not ASCII. A failed run, not help, so it
    /// exits non-zero.
    ///
    /// The empty name is passed as an argument of its own: clap takes the `=`
    /// off `-D=1`, which reaches here as the name `1`.
    ///
    /// Mutation: drop the identifier check on `-D`. Each is accepted, and the
    /// function-like one is read as `#define F(x) x`, a different directive.
    /// Mutation: ask `char::is_alphanumeric` of the characters after the
    /// first. `Aé` is accepted. Mutation: take an empty name for an
    /// identifier. `-D ""` is accepted.
    #[test]
    fn a_define_that_is_not_an_object_like_macro_is_refused() {
        for given in [
            &["safec", "-DF(x)=x", "a.c"][..],
            &["safec", "-D", "", "a.c"],
            &["safec", "-D1x", "a.c"],
            &["safec", "-Dé", "a.c"],
            &["safec", "-DAé", "a.c"],
        ] {
            let (kind, message) = macro_refusal(given);
            assert_eq!(kind, ErrorKind::ValueValidation, "{given:?}");
            assert!(message.contains("object-like"), "{given:?}: {message}");
        }
    }

    /// An `-U` of something that is not a name is refused the same way,
    /// an empty one among them.
    ///
    /// Mutation: drop the identifier check on `-U`. Both are accepted.
    #[test]
    fn an_undefine_that_is_not_a_name_is_refused() {
        for given in [&["safec", "-UF(x)", "a.c"][..], &["safec", "-U", "", "a.c"]] {
            let (kind, message) = macro_refusal(given);
            assert_eq!(kind, ErrorKind::ValueValidation, "{given:?}");
            assert!(message.contains("`-U "), "{given:?}: {message}");
        }
    }

    /// `-D N=` defines `N` as nothing, not as `1`.
    ///
    /// Mutation: take an empty value for no value. `N` is defined as `1`.
    #[test]
    fn a_define_with_an_empty_value_defines_the_name_as_nothing() {
        assert_eq!(
            macros_of(&["safec", "-DN=", "a.c"]),
            [CommandLineMacro::Defined {
                name: "N".to_owned(),
                value: String::new(),
            }]
        );
    }

    /// A value holding a line break is refused, since it would split the line
    /// it becomes into two directives.
    ///
    /// Mutation: drop the line-break check. It is accepted, and
    /// `preprocess` panics on it instead.
    #[test]
    fn a_define_whose_value_holds_a_line_break_is_refused() {
        for given in ["-DA=1\n#define B 2", "-DA=1\r"] {
            let (kind, message) = macro_refusal(&["safec", given, "a.c"]);
            assert_eq!(kind, ErrorKind::ValueValidation, "{given:?}");
            assert!(message.contains("line break"), "{given:?}: {message}");
        }
    }

    /// What `-D` and `-U` became, in order.
    fn macros_of(args: &[&str]) -> Vec<CommandLineMacro> {
        Cli::try_parse_in_order_from(args)
            .expect("the arguments parse")
            .into_options()
            .expect("the macros are read")
            .macros
    }

    /// `-D` and `-U` keep the order they were given in, as `cc` applies them,
    /// so `-U X -D X=2` ends with `X` defined: a hardened build's
    /// `_FORTIFY_SOURCE`. See ADR-0059.
    ///
    /// Mutation: sort the order by flag, every `-D` first. The first case ends
    /// with the `-U`. Mutation: leave `order` empty in
    /// `try_parse_in_order_from`. `into_options` panics that the order was not
    /// kept.
    #[test]
    fn defines_and_undefines_keep_the_order_they_were_given_in() {
        let defined = |value: &str| CommandLineMacro::Defined {
            name: "X".to_owned(),
            value: value.to_owned(),
        };
        let undefined = CommandLineMacro::Undefined {
            name: "X".to_owned(),
        };
        assert_eq!(
            macros_of(&["safec", "-UX", "-DX=2", "a.c"]),
            [undefined.clone(), defined("2")]
        );
        assert_eq!(
            macros_of(&["safec", "-DX=1", "-UX", "-D", "X=3", "a.c"]),
            [defined("1"), undefined, defined("3")]
        );
    }

    /// A name given to one flag twice is read twice, as `cc` reads it.
    ///
    /// Mutation: refuse a name already seen. Either case is refused.
    #[test]
    fn a_name_given_twice_to_one_flag_is_read_twice() {
        assert_eq!(macros_of(&["safec", "-UA", "-UA", "a.c"]).len(), 2);
        assert_eq!(macros_of(&["safec", "-DA", "-DA", "a.c"]).len(), 2);
    }

    /// A `Cli` holding `-D` or `-U` that was not parsed in order is refused
    /// rather than read with every `-D` first.
    ///
    /// Mutation: drop the assertion in `command_line_macros`. This is
    /// accepted, and the order it would have described is not the one given.
    #[test]
    #[should_panic(expected = "the order of `-D` and `-U` was not kept")]
    fn a_command_line_parsed_without_its_order_is_refused() {
        let _ = Cli::try_parse_from(["safec", "-UX", "-DX=2", "a.c"])
            .expect("the arguments parse")
            .into_options();
    }

    /// A named target is the target, on a host this compiler knows or one it
    /// does not, and no name is the host where the host is known.
    ///
    /// Mutation: refuse an unknown host before asking whether a target was
    /// named. The first assertion fails, which is the run the issue says must
    /// work. Mutation: ignore the host and default to `Target::ALL[0]`. The
    /// second fails, `x86_64-unknown-linux-gnu` not being that row.
    #[test]
    fn a_named_target_is_used_whatever_the_host_is() {
        let named = resolved_target(Some("wasm32-unknown-unknown"), "x86_64-unknown-linux-musl")
            .expect("a named target needs no host");
        assert_eq!(named.triple(), "wasm32-unknown-unknown");

        let host = resolved_target(None, "x86_64-unknown-linux-gnu")
            .expect("a known host is its own default");
        assert_eq!(host.triple(), "x86_64-unknown-linux-gnu");
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
