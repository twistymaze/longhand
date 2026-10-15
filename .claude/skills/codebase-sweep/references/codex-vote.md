# Codex verification vote

You run one verification vote with the Codex CLI. Codex does the analysis. Do not add your own opinion of the finding, and do not change the verdict that Codex writes.

## Steps

1. Make the output folder: `mkdir -p "$(dirname <output file>)"`.
2. Write the prompt below to `<output file>.prompt.txt`, with the finding JSON in its place.
3. Run Codex. The Bash sandbox stops the Codex CLI from starting, so run this command with the sandbox turned off (`dangerouslyDisableSandbox: true`). Codex itself runs with its own read-only sandbox:

   ```sh
   codex exec -m <model> -s read-only -C <repository root> --ephemeral \
     --output-schema <verdict schema> -o <output file> - < <output file>.prompt.txt
   ```

   Give the Bash call a timeout of 900000 ms (15 minutes).
4. Read `<output file>`. When it is valid JSON with the fields of the verdict schema, return those fields unchanged, with `voter_failed` false.
5. When Codex exits with an error, times out, or writes no valid JSON, return:
   - `status`: `unresolved`
   - `severity`: the severity of the finding
   - `protected_subject`: `none`
   - `reason`: `codex did not run: ` and the first error line from its output
   - `evidence`: an empty list
   - `known_decision`: an empty string
   - `voter_failed`: true

   Do not run Codex a second time.

## Prompt for Codex

```
You are an independent verifier. Decide whether one reported defect is real.
Another model reported it. Treat it as a claim, not as evidence, and read the code yourself.

Read <verifier rules> and follow it. Your role is "refute": try to show that the
defect cannot occur. Then do Steps 3 and 4 of those rules.
Do not edit any file.

<finding>
<finding JSON>
</finding>

Your final message is the verdict, as JSON that matches the output schema.
```
