# Lens: usage

The shard names dependencies (external packages, or files copied without changes) and the places in our code that call them. Do not look for defects inside the dependency. Find places where our code makes an assumption about the dependency that is not true.

## Method

1. For each call site, write down what our code assumes: the thread or runtime that a callback runs on, whether a call blocks, which errors it returns, when it panics or throws, units and formats of values, the lifetime of returned handles, behaviour on each platform that the project supports, and behaviour when it is called two times.
2. Check each assumption against the dependency's source code for the version that the project uses. The profile says where the source is (for example the Cargo registry or `node_modules`). Cite the dependency line in the evidence. Documentation is evidence only when it is for the same version.
3. Report the assumptions that are false and that can cause a failure in our code.

## For each finding

`file` and `line` are our call site. Put the dependency's path and line in `failure`.

## Do not report

Defects in the dependency that our code does not reach. A dependency version that is old, unless the old version causes a defect in our use.
