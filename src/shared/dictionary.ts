import { z } from 'zod'

export const dictionarySuggestionSchema = z.object({ word: z.string(), explanation: z.string() })
export const dictionaryEntrySchema = z.object({
  word: z.string(), ukPhone: z.string().nullable(), usPhone: z.string().nullable(),
  definitions: z.array(z.string()),
  forms: z.array(z.object({ name: z.string(), value: z.string() })),
  examples: z.array(z.object({ text: z.string(), translation: z.string() }))
})
export const dictionarySnapshotSchema = z.object({
  sessionId: z.string(), revision: z.number().int().nonnegative(), queryGeneration: z.number().int().nonnegative(),
  query: z.string(), mode: z.enum(['dictionary', 'ai']),
  status: z.enum(['loading', 'found', 'missing', 'error', 'cancelled']),
  entry: dictionaryEntrySchema.nullable(), suggestions: z.array(dictionarySuggestionSchema),
  error: z.string().nullable(), suggestionError: z.string().nullable()
})
export const studyBookSchema = z.object({ id: z.string(), name: z.string(), language: z.string() })
export type DictionarySuggestion = z.infer<typeof dictionarySuggestionSchema>
export type DictionarySnapshot = z.infer<typeof dictionarySnapshotSchema>
export type StudyBook = z.infer<typeof studyBookSchema>
