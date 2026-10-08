# Layout review rubric

You are reviewing screenshots of Elyra Workspace, a macOS desktop app for coding
agents (sidebar of projects and threads on the left, thread tabs and the
conversation in the middle, a tools panel on the right). Each screenshot is one
scene; its name says what state the app is in (window size, panel, theme).

Report only problems a user would notice. For each one give: the scene, where on
the screen, what is wrong, and how bad it is (high: unusable or hides content;
medium: looks broken; low: polish).

Look for:

1. **Overlap**: text or controls drawn over other text, controls or panel
   borders (for example tabs running into the tools panel).
2. **Clipping**: labels, buttons or text cut off mid-word without an ellipsis, or
   content hidden behind another element.
3. **Overflow**: something wider or taller than its container, pushing past a
   panel edge or out of the window.
4. **Truncation that loses meaning**: ellipses that hide the part a user needs
   (for example every tab reading "Kan du sjekke…").
5. **Alignment and spacing**: elements that should line up but don't, uneven
   gaps, controls stuck to an edge without padding.
6. **Contrast and theme**: text that is hard to read on its background, light
   elements left in a dark theme or the reverse, invisible icons.
7. **Empty or broken states**: blank panels without explanation, placeholder
   text that looks like a bug.

Ignore: content of the conversation itself, the test data's wording, and the
fact that some panels are empty because the test projects are small.

End with a short list of the problems worth fixing first. If a scene looks fine,
say so in one line.
