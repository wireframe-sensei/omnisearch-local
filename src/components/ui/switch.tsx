import { forwardRef } from "react";

interface SwitchProps extends React.InputHTMLAttributes<HTMLInputElement> {
  checked?: boolean;
  onCheckedChange?: (checked: boolean) => void;
}

const Switch = forwardRef<HTMLInputElement, SwitchProps>(
  ({ checked, onCheckedChange, className, ...props }, ref) => {
    return (
      <label className="inline-flex items-center cursor-pointer">
        <input
          ref={ref}
          type="checkbox"
          checked={checked}
          onChange={(e) => onCheckedChange?.(e.target.checked)}
          className="sr-only peer"
          {...props}
        />
        <div
          className={`relative w-11 h-6 rounded-full transition-colors duration-200 peer-focus:outline-none peer-focus:ring-2 after:content-[''] after:absolute after:top-0.5 after:left-0.5 after:bg-white after:rounded-full after:h-5 after:w-5 after:transition-transform ${
            checked
              ? "bg-blue-500 peer-focus:ring-blue-500 after:translate-x-5"
              : "bg-gray-400 peer-focus:ring-gray-400 after:translate-x-0"
          }`}
        />
      </label>
    );
  }
);

Switch.displayName = "Switch";

export { Switch };
