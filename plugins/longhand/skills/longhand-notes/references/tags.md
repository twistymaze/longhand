# Tags

A tag is a free text label on a note. A note has zero or more tags. The tags are in `longhand_tags` in `session.md`:

```yaml
longhand_tags:
- Finance
- Person::Nathan
```

- When the text has `::`, the part before it is the scope and the part after it is the value. `Person::Nathan` has the scope Person and the value Nathan. One note can have several tags with one scope.
- Two tags are the same tag when their tag keys are equal. The tag key is the text with the spaces at its ends and around each `::` removed, in lower case. `Team :: Ops` and `team::ops` are one tag.
- A tag cannot be empty. It cannot have a line break, a tab or another control character.
- The tag list in Longhand is every tag that is on at least one note. There is no other list of tags.

## Add or remove a tag

1. Find the notes with `list_notes.py`. Use `--not-busy` to skip busy notes.
2. Make a snapshot.
3. In each `session.md`, add or remove the item in `longhand_tags`. If the note has no `longhand_tags` key, add it at the end of the frontmatter.
4. Do not add a tag that the note has already with another case or other spaces. Use the text that the note has.
5. Compare.

## Rename or merge a tag

Change the text of the tag in each note that has it. When a note has the old tag and the new tag, keep only one of them. `check.py` gives a warning for two tags with one tag key.

## Use the existing texts

Before you add a tag, look at the tags that other notes have (`list_notes.py --json`). Use the same text for the same tag, for example `Person::Nathan`, not `person:: nathan` in one note and `Person::Nathan` in another.
