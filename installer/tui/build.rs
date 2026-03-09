fn main() -> Result<(), Box<dyn std::error::Error>> {
    let build = vergen_gitcl::BuildBuilder::default()
        .build_date(true)
        .build()?;
    let gitcl = vergen_gitcl::GitclBuilder::default().sha(true).build()?;

    vergen_gitcl::Emitter::default()
        .add_instructions(&build)?
        .add_instructions(&gitcl)?
        .emit()?;

    Ok(())
}
