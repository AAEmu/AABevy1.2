# Look conversion evidence

Pinned module: `<client12-unpacked>/x2game_unpacked.dll`, SHA256
`851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e`.

`python crates/aa-net/tests/fixtures/look-conversion/native_oracle.py` reruns nine
isolated Unicorn cases. It reads the original PE as data, asserts the hash, and
never loads it into Windows, opens a socket or accesses a live process. Python
pefile, capstone and unicorn are analysis-only tools, not Cargo dependencies.
Normal Rust tests use the committed byte fixtures and need neither the PE nor Python.

Three CS049 constructor/serializer cases execute read and write, with four actual
selected-slot caller cases. Two cleanup/Skinize cursor cases execute3942fb50 with
supplied actor/cursor/items/empty pending-map fixtures. The protected ordinary
planner39396e20 is intercepted to capture its inputs. Those inputs separately
execute native397cad20/397c85b0/397bfa90/397bfd50 serializers. This proves the
input/wire mapping, not execution of ordinary planner admission or live success.
Archive/BC/memset, converter validators, money and send/error endpoints are
explicitly qualified in native-goldens.json. Goldens are bodies without DD01 headers.
Zero IDs in convert_zero.bin are serializer fixtures, not valid request captures.

`native-disassembly.txt` retains exact instruction bytes. `source-provenance.json`
pins current lane and original C# commits with full-file hashes; `catalog.json`
records88 targeted SQLite rows and exact read-only queries. Source excerpts and
the original coordination proposal also remain under the ignored research path
`data/extract/aa-net/look-conversion-20261023/`.

No standalone success receipt is added. Existing SC090 tasks carry appearance
reason111/detail9, Skinize reason113/flags10 and powder/removal changes. Full
128-byte task detail qualification, native PRE-row projection and certainty
retirement remain unchanged. Generic source consumption can add a later reason40
receipt; clients must not compensate or assume a fixed receipt count.
