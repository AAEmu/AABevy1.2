# Read-only language_emptyWidget audit, 2026-10-23

**Finding: the original ACTABILITY_REFRESH_ALL closure has a latent source bug.
The current aa-ui receiver/payload convention agrees with native 1.2. No aa-ui
production fix is justified by this evidence.**

## Original bytes and exact failing path

Freshly extracted, using only prebuilt tools/aa-pak.exe:

- scriptsbin/x2ui/skill/tab_language.alb, 8,345 bytes, SHA256
  e63c897506993092981e5ee4ccdcccd9b1254fd06da0ed2a285ef5417032af37.
  It matches the existing data/extract/game copy byte for byte.
- Targeted baselib/baselib.alb and skill/main_window.alb. Hashes and bounded
  decompiled excerpts are in context.json.

tab_language.dis.txt preserves original line metadata. main/f1/f5 is the
ACTABILITY_REFRESH_ALL callback, original lines 167-189, **zero parameters**,
upvalues emptyWidget and actabilityInfos. Original line 178 executes:

    GETGLOBAL r7, "self"
    GETTABLE  r7, r7, "actabilityItem"
    GETTABLE  r7, r7, count

This is a global lookup, not a missing positional receiver or an upvalue. It is
reached when a current language row corresponds to an existing language item.
Original line 192, main/f1/f6, is the wrapper:

    function(this, event, ...) events[event](...) end

The wrapper does not pass `this` to the zero-parameter callback. Supplying an
extra receiver as payload cannot bind its global self lookup.

There is another independent original-script error: original line 188 calls
emptyWidget.countLabel:Update(), while CreateLanguagePage never creates or
assigns countLabel. No countLabel construction or assignment exists in this
chunk. The complete bytecode listing contains only the field read. Empty infos
reaches line 188 directly; a counterfactual global-self injection gets past 178
and then fails at 188. This control is evidence, not a proposed workaround.

The older data/extract/aa-lua/decompiled/x2ui/skill/tab_language.lua differs from
the original at the failing lookup (it says emptyWidget). Do not use that copy
to reason about the failing bytecode. Fresh unluac output and disassembly here
both say self.

## Native event, registration, and VM calling convention

native-source.txt contains fresh PE instructions and original PE hashes.

- Static initializer 3921487b pushes literal ACTABILITY_REFRESH_ALL at 399beb8c;
  39214880 selects slot 39d22e3c = 39d22ce8 + 0x55*4. The runtime table is zero
  before static initialization in the on-disk PE; this is expected.
- OnActability packet handler 391db5e0 reads row count at packet+0xd and rows at
  packet+0x10, then calls 3930bc60. The latter rebuilds proficiency rows and ends
  with 3930be5a `push 0x55`, 3930be5c `call 39424220`. That zero-payload emitter
  calls 3920fa60 at 3942422c. The inspected direct-call scan found this one
  callsite with push 0x55 immediately before that emitter.
- Native registration 39269420 binds SetHandler to 39264ff0; the binding calls
  392792e0, name lookup 3922c160, and handler-reference store 39279260. OnEvent
  is slot 8, widget+0x1a4. Original RegistUIEvent simply calls RegisterEvent for
  each event-table key; it does not wrap the function or change its environment.
- 3920fa60 reads the registered widget's OnEvent reference, uses BeginCall at
  script-system vtable+0x40, marshals via 3920d3b0, then EndCall at +0x44.
  The marshaller pushes exactly receiver ScriptAnyValue(type 6) and event-name
  ScriptAnyValue(type 5). Visibility does not gate this loop.
- Cry vtable 355703d0 maps +0x40 to 3550bc30 (registry function lookup), +0x60
  to 3550c440 (push parameter and increment count), and +0x44 to 3550af90 ->
  3550af20 (protected call). These functions use the Lua stack. They do not set
  global self or rebind a closure environment. By contrast, the separate Cry
  global setter at 3550c460 performs the global table write; this path never
  calls it. The executed trace calls the protected-call endpoint with two
  arguments, zero results, and error-handler stack index 1.

## Executed offline cases and limits

lua_cases.py: **8 expected-outcome cases passed** using original closure
instructions in Lua 5.1. Only serialization widths are widened in memory;
instruction bytes and line tables are retained and hashed. No Lua source
decompilation is executed. Controlled inputs are closure-upvalue tables and
X2Ability getters, without running or rendering a replacement UI constructor.

Cases: native-shaped zero payload; extra receiver payload; hidden receiver;
empty current infos; counterfactual global self only; normal modifier callback;
normal equipment callback; normal expert callback. The first three reproduce
178 -> 192, empty infos and counterfactual self reproduce 188 -> 192, and the
three neighboring callbacks update their item correctly without global self.

native_cases.py: **6 expected-outcome cases passed** executing actual native
name lookup/handler-reference storage, event dispatch/marshalling, Cry BeginCall,
parameter push and EndCall under Unicorn. Its protected-call endpoint invokes
the original Lua handler/refresh closures. Cases: no subscriber; registered
shown widget; registered hidden widget (native shown byte +ba = 0); no handler;
zero-row proficiency rebuild producing refresh; setting handler nil releasing
its reference. Both registered delivery cases and the rebuild reproduce 178.

Explicit controlled native endpoints: manager admission/list setup, proficiency
reset/post-rebuild state, profiling/log formatting, Lua stack primitives,
table refcounts, binding-event registration, and protected call. This does not
execute the original float32 Lua VM or prove whole-client startup packet timing.
It does execute the native code that selects the closure and decides every
callback argument. Details and traces are in lua-cases.json and native-cases.json.

## Current application exposure and proposed ownership

The supplied log's 178 -> 192 stack uniquely selects ACTABILITY_REFRESH_ALL.
Current aa-ui/src/lua/mod.rs:351-356 and :387-392 already deliver the native
(receiver,eventName) convention for this zero-payload event.

aa-client/src/hud/crafting/expert.rs has two concrete producers: line 305 on a
last=true received snapshot; line 354 on first HUD activation when its state is
already loaded. The latter clears historical events and re-emits a refresh
after widgets have been constructed. Native code inspected here emits on
packet handling, with no analogous startup replay in that path. The supplied
log does not identify which application callsite fired; first-activation replay
is a source-supported candidate, not a runtime-proven attribution.

No owned aa-ui fix is proposed. Do not globally synthesize self, add hidden-widget
filters, change this event's payload, suppress/whitelist the exception, or patch
the original bytecode based on this audit. None follows from native behavior.

Concrete root-owned follow-up proposal, requiring separate coordination:
investigate eliminating the synthetic first-activation replay **only when the
constructed language/proficiency widgets already reflect the final received
snapshot**. Constructors query proficiency getters, and crafting::refresh copies
the received state before expert.active; prebuilt-stage snapshots can be older,
so do not assume this freshness invariant or just delete every refresh. Keep
real post-construction packet notifications. This may avoid the entry exposure;
it does not repair the latent source bug for later real refreshes. A policy for
repairing shipped-script defects would be a separate decision with both 178 and
188 addressed explicitly, not an inferred native compatibility shim.

No Cargo, build, server, original-client process access, production edit, root
mode-bar edit, tracked docs edit, or commit was performed. All retained audit
artifacts are ignored files under data/extract/aa-ui/language-self. No complete
session log or credential values were retained in the evidence.
