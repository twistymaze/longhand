# Built-in prompts

Longhand has two built-in summary prompts. They have no file in the storage folder, and you cannot change them. This file has their text, so that you can tell the user what a built-in prompt does, and make a changed copy of it.

The text is the text of the current Longhand version. The user's Longhand can be older or newer, and then its text can be different. Settings in Longhand always shows the text of the installed version.

Each block below is the body of a prompt file: the prompt text, the line `<!-- longhand: title instructions -->`, and the title instructions. To make a changed copy, follow "Make a changed copy of a built-in prompt" in `prompts.md`.

## Topic summary (`topic-summary`)

The default prompt. It groups the summary by topic, not by time.

```text
Write the summary in Markdown, grouped by topic, not in time order. Do not add a title. Do not show times.

Start with a short overview paragraph of two or three sentences.

Then write one section for each topic, with a "## " heading that names it. Start the section with one line that sets up the topic. Then write bullets that each start with a short bold label, such as "**Decision:**" or "**Budget:**". Put the specific details in bullets nested below.

State the facts, not that people talked about them. Write "the launch moves to May 12 and the budget rises to $40,000", not "the team discussed the schedule". Keep numbers, amounts, names, dates and specific items. Leave out small talk.

The notes are what the person who recorded the session found important. Give their subjects more weight and more detail, but also cover every other significant topic.

End with an "## Open questions" section and a "## Follow-ups" section that gives the owner of each follow-up. Leave out a section that has nothing in it.
<!-- longhand: title instructions -->
Start the title with the type of meeting, then a colon, then the main topics, for example "1:1 Meeting: Hiring plan and Q3 budget". Use one of these types when it fits: 1:1 Meeting, Customer Meeting, Internal Meeting, Team Meeting, Brainstorming Meeting, Planning Meeting, Status Meeting, Interview. When another type fits better, use a short name for it, such as "Game Session" or "Class".

Find the type from the transcript and the notes: how many people take part, how they relate to each other and what kind of conversation it is. When the type is not clear, write only the topics, with no type and no colon.

Keep the title to about ten words.
```

## Play-by-play summary (`play-by-play`)

The summary keeps the time order, and each key point has its time.

```text
The notes are one document that the person who recorded the session typed. They mark what that person found important. They have no times and do not point to a moment in the transcript. Give the subjects that the notes mention more weight and more detail. Still report every significant decision, disagreement, plan or piece of information that the notes do not mention: a subject without a note is not unimportant.

Write the summary in Markdown: a short overview paragraph, then a "Key points" list in time order, each point with its [hh:mm:ss] time, then "Decisions" and "Open questions and follow-ups" lists when there are any. Do not add a title.
<!-- longhand: title instructions -->
Start the title with the type of meeting, then a colon, then the main topics, for example "1:1 Meeting: Hiring plan and Q3 budget". Use one of these types when it fits: 1:1 Meeting, Customer Meeting, Internal Meeting, Team Meeting, Brainstorming Meeting, Planning Meeting, Status Meeting, Interview. When another type fits better, use a short name for it, such as "Game Session" or "Class".

Find the type from the transcript and the notes: how many people take part, how they relate to each other and what kind of conversation it is. When the type is not clear, write only the topics, with no type and no colon.

Keep the title to about ten words.
```
