import { z } from "zod";

export const SystemInfoSchema = z.object({
os: z.string().min(1),
architecture: z.string().min(1),
hostname: z.string().min(1),
appVersion: z.string().min(1),
});

export type SystemInfo = z.infer<typeof SystemInfoSchema>;

