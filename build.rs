fn main() -> Result<(), Box<dyn std::error::Error>> {
    vergen::EmitBuilder::builder().git_sha(false).emit()?;

    Ok(())
}
