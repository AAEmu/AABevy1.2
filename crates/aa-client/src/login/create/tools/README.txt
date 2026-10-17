Compare tools of the create screen (Python 3 with Pillow and numpy; run from the repo root, images
in shots/create/): gen_all.py + run_create.sh capture 3 steps x 8 models, sheet.py makes
reference|ours sheets, align3.py registers our render onto the capture (scale + shift),
bbox.py measures our silhouette against shots/create/bg (AA_CREATE_NO_SUBJECT=1 run), sbs.py crops
two images side by side.
