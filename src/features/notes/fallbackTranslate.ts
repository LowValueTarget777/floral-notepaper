import type { TFunction } from "i18next";

export const fallbackTranslate = ((key: string, options?: { defaultValue?: string }) =>
  String(options?.defaultValue ?? key)) as TFunction;
