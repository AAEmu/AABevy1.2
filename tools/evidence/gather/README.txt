Natural gathering evidence (2026-10-30)

* capture-plan.txt     what the original 1.2 client must be recorded doing (the original contract).
* capture-results.txt  the 2026-10-30 native capture results this slice used: confirmed herb and
                       fake-use values, the bag-full/no-labor/range outcomes, and what was not captured.
* replay.txt           the Bevy side script (tools/gather_replay.py runs it with --headless).
* bevy.json            written by the replay: one record per gathering cast, decoded from the
                       server's fields trace. Labor, error and item rows belong to the cast that is
                       running (its CSStartSkill to its SCSkillEnded or refusal), not to earlier casts.
                       item_tasks lists what SCItemTaskSuccess put into the bag as
                       {template_id, amount, grade}, decoded with the aa-net layout
                       (crates/aa-net/src/items.rs `ItemTasks::read`); skill_started.result is the
                       refusal code (0 = the cast started).
* report.txt           written by the replay: side-by-side status (or "native.json is missing").
                       It compares target kind, cast time, the refusal code (skill_started.result),
                       phase changes, removal, labor, item_tasks and errors; a field the native
                       case does not list is skipped. item_tasks is compared as one total per
                       template id and grade, not as packet bytes: bag slots and item db ids differ
                       from run to run.
                       `python tools/gather_replay.py --self-test` runs the offline tests.
* native.json          the 1.2 capture lives under <capture-root> (see capture-results.txt);
                       it is not copied here and screenshots stay local. It is row-based (one row
                       per packet or sample), not the per-case schema of capture-plan.txt, so the
                       replay still reports "native.json is missing": compare by hand against
                       capture-results.txt.

The comparison keys a case by its skill id: 11106 ore, 11107 rock, 13802 sheep pluck,
13970 sheep butcher, 13990 herb/grass. 11642 is the legacy shear case (template 307/1568).

No game files or real-client screenshots are tracked here. Screenshots go to
reference/gather/ (git-ignored); only JSON/report text is committed.
