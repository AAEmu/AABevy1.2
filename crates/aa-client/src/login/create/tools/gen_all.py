# Command line for one run that captures step 1, 2 and 3 of all eight models into shots/create/cmp/ (or $GEN_DIR):
#   tools/run_create.sh shots/create/all.log --goto create --account bevycreate $(python tools/gen_all.py)
# (--create-offline instead of --goto create takes the shots without entering a world)
# (then: python tools/sheet.py 2 sheet2.png 420 100 1400 1000; python tools/align3.py 2 460 20 1380 760)
import sys, shlex, os
OUT = os.environ.get('GEN_DIR', 'shots/create/cmp')
models = [(r,g) for r in ['nuian','elf','hariharan','ferre'] for g in ['male','female']]
only = sys.argv[1:] 
args = []
t = 4.0
RC = {'nuian':'RACE_NUIAN','elf':'RACE_ELF','hariharan':'RACE_HARIHARAN','ferre':'RACE_FERRE'}
for r,g in models:
    if only and f"{r}_{g}" not in only: continue
    gi = 2 if g=='female' else 1
    sel = f"for k=1,#RACE_TYPE do if RACE_TYPE[k]=={RC[r]} then step1.content.raceBtn[k].OnClick() end end step1.content.rightPanel.genderBtn[{gi}].OnClick()"
    args += ["--clua", sel, f"{t}"]
    args += ["--cshot", f"{OUT}/step1_{r}_{g}.png", f"{t+2.5}"]
    args += ["--clua", "characterCreate.OnNextStep()", f"{t+3}"]
    args += ["--cshot", f"{OUT}/step2_{r}_{g}.png", f"{t+6.5}"]
    args += ["--clua", "characterCreate.OnNextStep()", f"{t+7}"]
    args += ["--cshot", f"{OUT}/step3_{r}_{g}.png", f"{t+13.5}"]
    args += ["--clua", "characterCreate.OnPrevStep()", f"{t+14}"]
    args += ["--clua", "characterCreate.OnPrevStep()", f"{t+16.5}"]
    t += 19
args += ["--exit-after", f"{t+20}"]
print(" ".join(shlex.quote(a) for a in args))
