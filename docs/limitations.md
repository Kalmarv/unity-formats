# Guarantees and limitations

## What it guarantees

- Reads are bounds-checked, and a file it does not understand is refused with an `Error` that says what it found, for example `SerializedFile version 16 (read: 17..=22)`, `header file size 9758532, file has 30 bytes` or `<n> bytes wanted at offset <p>, <m> left`. Only SerializedFile versions checked against real files are read; anything else is `Error::Unsupported`, never a guess.
- Library code uses lints to deny panicking indexing, unchecked arithmetic and truncating casts, so a malformed file produces an `Error`, not a panic. The test suite checks truncated headers and unsupported versions on synthetic bytes; it does not fuzz.
- A reader returns what the file stores. Values keep their stored units and Unity's coordinate frame (no handedness change, resampling or colour-space change), and pixels stay as stored. Where a module substitutes a default for a field the file may omit, its docs say so.
- The library never writes to disk. It reads only the files in the folder it is given (a player's data folder and its `Resources` folder), the bundles it is told to open and the `.resS` files its objects name there (or the stream nodes of the bundle they are in). A streamed resource's path is reduced to a file name in that folder, or to a node of the bundle. `unsafe` code is denied, and only `src/player.rs`, `src/resource.rs` and `src/bundle.rs` touch the file system.

## What it does not do

- It only reads: nothing here writes SerializedFiles, bundles or any other Unity format.
- It reads UnityFS bundles of formats 6 to 8, their block table with the directory (header flag `0x40`, with or without the `0x200` padding flag) or at the end of the file (`0x80`), and blocks stored, LZMA, LZ4 or LZ4HC. Bundles with LZHAM blocks or of another format version are refused. Addressables catalogs are not read. SerializedFiles of container versions other than 17 to 22 are refused.
- It ships no type tree package; packages uncompressed or compressed with LZ4, LZMA or Brotli are read.
- In a file without type trees, a MonoBehaviour's script fields are returned as bytes. A file that stores its trees (a bundle's) decodes them with `Player::script_fields`. `[SerializeReference]` managed references are read by their class's tree among the file's reference types; one whose class the file has no tree for is refused by name.
- Textures: every format of Unity's `TextureFormat` is named but PVRTC (30 to 33), the 3DS forms of ETC (60, 61) and the retired ids, which are refused. Crunched DXT1 and DXT5 (Unity 2017.3 on) unpack to their DXT blocks; crunched ETC formats and crunch's original format (before Unity 2017.3) are refused. No format is decoded to RGBA; the data is only split into mip levels.
- Meshes with compressed skin weights are refused. Of AnimationClips, Mecanim int curves (`isIntCurve`), compressed legacy clips and legacy Euler curves of a rotation order other than Mecanim's are refused; legacy clips and object-reference (PPtr) curves are read.
- A streamed AnimationClip whose frames name a curve past its stated curve count (some of PEAK's 6000.3.15f1 clips) is refused as `Error::Unsupported`, not guessed at.
- The Universal Render Pipeline asset layout is verified only for Unity 6000.3.12f1 and 6000.3.15f1 players; an asset of another version is refused by name.
- It does not run, render or simulate anything. Animation data, shaders and materials are read as stored, and interpreting them is up to the caller.
- It contains no game files, and no test requires you to share any.
