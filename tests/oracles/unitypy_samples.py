"""UnityPy's reading of bundles, for tests/public_samples.rs (`oracles.unitypy.general/samples.json`): per bundle, its
format, header flags and nodes (path, size, SHA-256 of the bytes), and per SerializedFile node its version, Unity
version and objects (path id, class id, byte size). JSON to stdout. Made with UnityPy 1.25.4 over UnityPy's own test
samples (`oracles.corpus.general/unitypy-samples`):
usage: uvx --from UnityPy==1.25.4 python -I unitypy_samples.py BUNDLE... > samples.json"""
import hashlib, json, sys
import UnityPy
from UnityPy.files import BundleFile, SerializedFile

out = {}
for p in sys.argv[1:]:
    env = UnityPy.load(p)
    (bundle,) = env.files.values()
    assert isinstance(bundle, BundleFile)
    nodes = []
    for name, f in bundle.files.items():
        if isinstance(f, SerializedFile):
            raw = bytes(f.reader.bytes)
            nodes.append({"path": name, "serialized": True, "version": f.header.version,
                          "size": len(raw), "sha256": hashlib.sha256(raw).hexdigest(),
                          "unity": f.unity_version,
                          "objects": [[o.path_id, o.class_id, o.byte_size] for o in f.objects.values()]})
        else:
            b = bytes(f.bytes)
            nodes.append({"path": name, "serialized": False, "size": len(b),
                          "sha256": hashlib.sha256(b).hexdigest()})
    out[p.rsplit("/", 1)[-1]] = {"format": bundle.version, "flags": int(bundle.dataflags), "nodes": nodes}
json.dump(out, sys.stdout, indent=1)
