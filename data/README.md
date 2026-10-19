# Data on tiered-brains

This branch starts without previous training runs, trained champions, optimizer
states, experiment results, recordings or campaign packages. New generated
training output is ignored by Git.

`brains/from_replays` retains the authored baseline brains, reference catalogs,
compiled maps, provenance and scenario configurations used by tests and examples.
These are reference inputs, not learned champions. Retained inputs were copied
without modification. Local `replay/catalog*.json` files are also reference inputs.

Previous committed training results remain on `main` and the annotated tag
`minibar-training-radar-scouts-20261019`. The full previous local data tree,
including ignored artifacts, is archived under
`.quality-work/training-archive-before-tiered-brains-20261019/data`.
Two open dashboard log files also remain at their old local paths until the
existing dashboard process closes them; neither is tracked by Git.

Historical training guides describe the saved checkpoint. A new tiered-brain
campaign should start with fresh inputs and its own output directory.
