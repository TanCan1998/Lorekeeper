type Text = (zh: string, en: string) => string

export function chapterLabel(chapterNumber: number, text: Text): string {
  return text(`第 ${chapterNumber} 章`, `Chapter ${chapterNumber}`)
}

export function chapterTabName(chapterNumber: number, title: string, text: Text): string {
  const label = chapterLabel(chapterNumber, text)
  return title ? `${label} · ${title}` : label
}
