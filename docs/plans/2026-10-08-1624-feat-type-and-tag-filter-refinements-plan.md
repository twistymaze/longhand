---
title: Type and Tag Filter Refinements - Plan
type: feat
date: 2026-10-08
topic: type-and-tag-filter-refinements
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Type and Tag Filter Refinements - Plan

## Goal Capsule

- **Objective:** On Home, each type and tag control shows only when it gives information. A user with many tags can find and choose a tag quickly. A user who uses note types can see which notes are in General.
- **Product authority:** This Product Contract. The user confirmed the scope in the brainstorm on 2026-10-08. It refines [Note Types and Tags](2026-10-07-1940-feat-note-types-and-tags-plan.md) (R19, R22) and does not change its other decisions.
- **Open blockers:** None.

---

## Product Contract

### Summary

Home treats note types as in use when the type list has a type other than General. Only then does Home show the type filter and a General label on General notes. Home shows the Tags menu only when a note has a tag. The Tags menu gets a field that filters the list, groups the tags by scope, and shows each tag as its coloured label.

### Problem Frame

Note types and tags arrived in PR #45. With daily use, three problems are clear.

A user who keeps every note in General sees "All Types | General" on Home. Both choices show the same notes. A user with no tags sees a Tags menu that holds only "No tags".

A user with many scoped tags gets a menu of plain text such as `Person::Andrew Wein…`. The `::` and the repeated scope make the list hard to scan, and the 240 px menu cuts off long names. The list will grow as the user tags more people.

A user with note types who sometimes uses General sees no type label on General notes. The row looks as if the type is missing, not as if the note is in General.

### Key Decisions

- **Note types are in use when the type list has a type other than General.** A type that the user made in Settings and that has no notes counts. A user who makes a type intends to use more than one type. (session-settled: user-directed — chosen over a rule that hides the filter when the choices give the same notes: the user wants the filter as soon as they make a type.) Governs R1, R2, R4.
- **The Tags menu is a grouped list with a filter field and the real tag labels.** (session-settled: user-directed — chosen over a grouped list with scope headings and over one submenu for each scope, after a sketch of all three.) Governs R6, R7, R8, R9.
- **The General label looks like the other type labels.** One look for all types is easier to read than a special case, although General has no type folder. Governs R4.
- **Tag colours do not change.** A tag with no scope is a neutral label, and each scope has its own colour. This already works for users with no scoped tags, only scoped tags, or a mix.

### Requirements

**Type filter**

- R1. Home shows the type filter only when note types are in use: the type list has at least one type other than General.
- R2. When the type filter shows, it lists "All", General and every other type in the type list, with or without notes. Today, General is a choice only when a listed note is in General. That condition is removed.
- R3. When a filter control is hidden, Home drops the choice that it held, so Home shows all notes for that control.

**General on Home rows**

- R4. When note types are in use (R1), each Home row of a General note shows a General type label. When they are not in use, Home rows show no type label for General notes, as today.

**Tags menu**

- R5. Home shows the Tags menu only when at least one note has a tag.
- R6. The Tags menu starts with a text field. Typing in the field shows only the tags whose scope or value contains the text, without regard to case.
- R7. The menu lists the tags with no scope first, then the scoped tags. Scopes sort A–Z, and the values in each scope sort A–Z. Tags with no scope sort A–Z.
- R8. Each menu item shows the tag as the same label that Home rows show, with the scope colour, and a check mark when the tag is chosen.
- R9. The menu stays open after the user chooses or clears a tag, so the user can choose several tags in turn.
- R10. The menu works from the keyboard: the field has the focus when the menu opens, the arrow keys move through the shown tags, Return chooses or clears the tag that has the focus, and Escape closes the menu.

### Acceptance Examples

- AE1. **Covers R1, R5.** **Given** every note is in General and no note has a tag, **when** the user opens Home, **then** Home shows no type filter and no Tags menu.
- AE2. **Covers R1, R2.** **Given** every note is in General, **when** the user adds the type "Pathfinder" in Settings, **then** Home shows the type filter with "All", "General" and "Pathfinder".
- AE3. **Covers R4.** **Given** the type list has "One-on-One" and a note is in General, **when** the user looks at Home, **then** that note's row shows a General type label.
- AE4. **Covers R3.** **Given** `follow-up` is the only tag in the library and the filter shows it, **when** the user removes `follow-up` from the only note that has it, **then** the Tags menu and the chosen-tag label go away, and Home shows every note for the chosen type.
- AE5. **Covers R6, R7.** **Given** the tags `draft`, `Person::Dana Ortiz` and `Topic::Roadmap`, **when** the user types "da" in the field, **then** the menu shows `draft` and then `Person::Dana Ortiz`.
- AE6. **Covers R9.** **Given** the Tags menu is open, **when** the user chooses two tags in turn, **then** the menu stays open, both show a check mark, and Home shows only the notes that have both tags.

### Scope Boundaries

- The note page's type menu and "+ Tag" picker do not change.
- No change to the filter logic (one type and all chosen tags) or to the segmented control and its overflow menu.
- No new tag colours, and no colours that the user chooses for a scope.
- No submenus and no scope-only filter.

### Outstanding Questions

**Deferred to Planning**

- What the menu shows when the field text matches no tag.
- Whether the menu needs more width than 240 px for long labels, or the labels cut off with an ellipsis.

### Sources / Research

- `src/home/FilterBar.tsx` — the filter bar, the type segments and the Tags menu.
- `src/home/filter.ts` — `hasGeneralNote` and `keepExisting` (R3 already drops a choice that no longer exists).
- `src/home/NoteLabels.tsx` — Home row labels. It shows no label for a `null` type.
- `src/lib/Labels.tsx` — `TagLabel`, `TypeLabel` and the scope colours.
- `src/session/SessionMenu.tsx` — `Menu`. A menu with no submenu scrolls.
- `src/lib/TagPicker.tsx` — the "+ Tag" combobox, a possible model for the filter field.
