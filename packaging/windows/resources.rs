// Windows resources for the executables' build script (`include!`d): version
// information naming AutoSplitter and its version, the icon and the manifest.

fn windows_resources(version: &str, description: &str, icon: &std::path::Path, manifest: &std::path::Path) {
    let mut numbers: Vec<u16> = version.split('.').map(|n| n.parse().expect("version is not x.y.z")).collect();
    numbers.resize(4, 0);
    let numbers = numbers.iter().map(u16::to_string).collect::<Vec<_>>().join(",");
    // rc and windres both take forward slashes, which need no escaping.
    let path = |p: &std::path::Path| p.display().to_string().replace('\\', "/");
    let rc = format!(
        r#"1 ICON "{icon}"
1 24 "{manifest}"
1 VERSIONINFO
FILEVERSION {numbers}
PRODUCTVERSION {numbers}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "CompanyName", "AutoSplitter"
            VALUE "FileDescription", "{description}"
            VALUE "FileVersion", "{version}"
            VALUE "LegalCopyright", "Copyright 2026 Kim Attree"
            VALUE "ProductName", "AutoSplitter"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#,
        icon = path(icon),
        manifest = path(manifest),
    );
    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("autosplitter.rc");
    std::fs::write(&out, rc).unwrap();
    embed_resource::compile(&out, embed_resource::NONE).manifest_required().expect("failed to compile Windows resources");
}
