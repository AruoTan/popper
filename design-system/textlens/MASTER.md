# TextLens UI polish — design notes

**Product:** desktop selection assistant (Tauri + React)  
**Style:** AI-native minimal chrome, native system fonts, existing token set  
**Motion:** 150–220ms fades/slides; pulse only for live thinking / dirty save  
**Density:** standard settings shell; dense result panel

## Tokens (keep project `:root`, do not force purple AI palette)

- Accent / focus: `--accent`, `--accent-soft`
- Surfaces: `--surface`, `--surface-solid`, `--surface-muted`
- Text: `--text`, `--text-secondary`, `--text-tertiary`
- Danger / success for stop + copy-done

## Surfaces

| Surface | Goals |
|---------|--------|
| Settings | Progressive disclosure, numbered provider setup, dirty save emphasis, section enter motion |
| Result | Streaming wait affordance, live thinking auto-expand, stop emphasis, reduced-motion safe |

## Anti-patterns avoided

- Heavy decorative animation
- Placeholder-only labels
- Instant 0ms state changes without feedback
- Emoji icons (Lucide only)
