import { File, FileText, FileType2, FileSpreadsheet, Presentation } from "lucide-react";
import { cn } from "@/lib/utils";

interface FileIconProps {
  extension: string;
  className?: string;
}

export function FileIcon({ extension, className }: FileIconProps) {
  const iconClassName = cn("text-muted-foreground", className);

  switch (extension) {
    case "txt":
    case "md":
    case "docx":
      return <FileText className={iconClassName} />;
    case "pdf":
      return <FileType2 className={iconClassName} />;
    case "xlsx":
      return <FileSpreadsheet className={iconClassName} />;
    case "pptx":
      return <Presentation className={iconClassName} />;
    default:
      return <File className={iconClassName} />;
  }
}
