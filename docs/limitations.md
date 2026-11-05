# Guarantees and limitations

## What it guarantees

- Reads are bounds-checked, and a file it does not understand is refused with an `Error` that says what it found, for example `SerializedFile version 19 (read: 21..=22)`, `header file size 9758532, file has 30 bytes` or `<n> bytes wanted at offset <p>, <m> left`. Only SerializedFile versions checked against real files are read; anything else is `Error::Unsupported`, never a guess.
- Library code uses lints to deny panicking indexing, unchecked arithmetic and truncating casts, so a malformed file produces an `Error`, not a panic. The test suite checks truncated headers and unsupported versions on synthetic bytes; it does not fuzz.
- A reader returns what the file stores. Values keep their stored units and Unity's coordinate frame (no handedness change, resampling or colour-space change), and pixels stay as stored. Where a module substitutes a default for a field the file may omit, its docs say so.
- The library never writes to disk. It reads only the files in the folder it is given (a player's data folder and its `Resources` folder), the bundles it is told to open and the `.resS` files its objects name there (or the stream nodes of the bundle they are in). A streamed resource's path is reduced to a file name in that folder, or to a node of the bundle. `unsafe` code is denied, and only `src/player.rs`, `src/resource.rs` and `src/bundle.rs` touch the file system.

## What it does not do

- It only reads: nothing here writes SerializedFiles, bundles or any other Unity format.
- It reads UnityFS bundles of format 8 whose block table is stored with the directory (header flag `0x40`, with or without the `0x200` padding flag) and whose blocks are stored, LZ4 or LZ4HC. Bundles with LZMA or LZHAM blocks, a block table at the end of the file or another format version are refused. Addressables catalogs are not read. SerializedFiles of container versions other than 21 and 22 are refused.
- It ships no type tree package. LZMA- and Brotli-compressed packages are refused (re-pack them uncompressed or with LZ4).
- In a file without type trees, a MonoBehaviour's script fields are returned as bytes. A file that stores its trees (a bundle's) decodes them with `Player::script_fields`. `[SerializeReference]` managed references are refused.
- Textures: eleven formats are named (Alpha8, ARGB4444, RGB24, RGBA32, DXT1, DXT5, BC4, BC5, BC6H, BC7, R8). Crunched formats are refused, and no format is decoded to RGBA; the data is only split into mip levels.
- Meshes with compressed skin weights, legacy AnimationClips and clips with object-reference curves are refused.
- A streamed AnimationClip whose frames name a curve past its stated curve count (some of PEAK's 6000.3.15f1 clips) is refused as `Error::Unsupported`, not guessed at.
- The Universal Render Pipeline asset layout is verified only for Unity 6000.3.12f1 and 6000.3.15f1 players; an asset of another version is refused by name.
- It does not run, render or simulate anything. Animation data, shaders and materials are read as stored, and interpreting them is up to the caller.
- It contains no game files, and no test requires you to share any.
