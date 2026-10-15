# Lens: seam

A seam shard is a flow that crosses several modules. Each module can be correct alone while the flow fails. Find those failures. Other lenses examine each module alone; your work is the space between the modules.

## Method

1. Read the shard's `intent` and follow the flow through each file in order. Write down what each side assumes about the other: data shape, units, order of calls, which side owns a file or a state, which errors can arrive.
2. Attack each assumption:
   - **Contract mismatch:** one side sends a value that the other side does not expect, or reads a return value in a different way.
   - **Order:** a step that assumes another step has run, without anything that makes it so. A callback that runs before setup ends.
   - **Shared state:** two parts that read and write the same file, record, or state without coordination.
   - **Error contract:** one side returns an error that the other side does not handle, or handles as success.
   - **Cascade:** a first failure that makes the next step act on partial data, and so on, until the user loses something.
   - **Normal use that goes wrong:** the same action two times quickly, an action during a transition (while saving, while processing, during an update), or the maximum valid input.
3. Report each failure as a scenario with a title that names the failure, for example "Stop during summary: the new transcript is saved to the previous note".

## For each finding

`trigger` gives the first event. `failure` lists each step with its file and line, then the final state.
