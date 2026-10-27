use cli_command_kit::Mode;
use einstar_vega_cli::cli;

#[test]
fn usage_documents_cover_native_interface_and_generated_help_queries() {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/vega-cli");
    let mut text = std::fs::read_to_string(folder.join("SKILL.md")).unwrap();
    for name in ["commands.md", "configuration.md", "outputs.md", "help.md"] {
        text.push_str(&std::fs::read_to_string(folder.join("references").join(name)).unwrap());
    }
    let omissions =
        cli_command_kit::skill_omissions(&cli::command(Mode::Agent), &cli::help_options(), &text);
    assert!(omissions.is_empty(), "{}", omissions.join("\n"));
    let queries = std::fs::read_to_string(folder.join("references/help.md")).unwrap();
    for line in queries
        .lines()
        .filter(|line| line.starts_with("- `vega-cli "))
    {
        let query = line.trim_start_matches("- `").trim_end_matches('`');
        for mode in [Mode::Agent, Mode::Human] {
            let error = cli::command(mode)
                .try_get_matches_from(query.split_whitespace())
                .unwrap_err();
            assert!(!error.use_stderr(), "{query}: {error}");
            assert_eq!(error.to_string().starts_with("ALERT:"), mode == Mode::Agent);
        }
    }
}
