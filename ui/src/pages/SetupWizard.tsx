import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { AlertCircle, Check, Eye, EyeOff } from "lucide-react";
import AuthSplitLayout, { authButtonClass, authInputClass } from "@/components/auth/AuthSplitLayout";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Field, FieldError, FieldLabel } from "@/components/ui/field";
import { Spinner } from "@/components/ui/spinner";
import { Input } from "@/components/ui/input";
import { cn } from "cn";
import { useAppDispatch, useAppSelector } from "@/store/hooks";
import { clearSetupError, submitSetup } from "@/store/setupSlice";

interface FormData {
  username: string;
  password: string;
  confirmPassword: string;
  rootPassword: string;
  confirmRootPassword: string;
  email: string;
}

interface FormErrors {
  username?: string;
  password?: string;
  confirmPassword?: string;
  rootPassword?: string;
  confirmRootPassword?: string;
  email?: string;
}

const STEP_TITLES: Record<1 | 2 | 3, string> = {
  1: "Create DBA account",
  2: "Set DBA password",
  3: "Set root password",
};

const STEP_DESCRIPTIONS: Record<1 | 2 | 3, string> = {
  1: "Create the first administrator account for this server.",
  2: "Choose a secure password for your DBA account.",
  3: "Set the root password used for system-level administration.",
};

export default function SetupWizard() {
  const dispatch = useAppDispatch();
  const navigate = useNavigate();
  const { isSubmitting, setupComplete, error, createdUsername } = useAppSelector((state) => state.setup);

  const [formData, setFormData] = useState<FormData>({
    username: "",
    password: "",
    confirmPassword: "",
    rootPassword: "",
    confirmRootPassword: "",
    email: "",
  });
  const [formErrors, setFormErrors] = useState<FormErrors>({});
  const [showPassword, setShowPassword] = useState(false);
  const [showRootPassword, setShowRootPassword] = useState(false);
  const [step, setStep] = useState<1 | 2 | 3>(1);

  useEffect(() => {
    if (error) {
      dispatch(clearSetupError());
    }
  }, [error, formData, dispatch]);

  useEffect(() => {
    if (!setupComplete) {
      return;
    }
    const timer = setTimeout(() => {
      navigate("/login", { replace: true });
    }, 3000);
    return () => clearTimeout(timer);
  }, [setupComplete, navigate]);

  const validateStep1 = (): boolean => {
    const errors: FormErrors = {};

    if (!formData.username.trim()) {
      errors.username = "Username is required";
    } else if (formData.username.toLowerCase() === "root") {
      errors.username = "Cannot use 'root' as username";
    } else if (formData.username.length < 3) {
      errors.username = "Username must be at least 3 characters";
    }

    if (formData.email && !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(formData.email)) {
      errors.email = "Invalid email format";
    }

    setFormErrors(errors);
    return Object.keys(errors).length === 0;
  };

  const validateStep2 = (): boolean => {
    const errors: FormErrors = {};

    if (!formData.password) {
      errors.password = "Password is required";
    } else if (formData.password.length < 8) {
      errors.password = "Password must be at least 8 characters";
    }

    if (formData.password !== formData.confirmPassword) {
      errors.confirmPassword = "Passwords do not match";
    }

    setFormErrors(errors);
    return Object.keys(errors).length === 0;
  };

  const validateStep3 = (): boolean => {
    const errors: FormErrors = {};

    if (!formData.rootPassword) {
      errors.rootPassword = "Root password is required";
    } else if (formData.rootPassword.length < 8) {
      errors.rootPassword = "Root password must be at least 8 characters";
    }

    if (formData.rootPassword !== formData.confirmRootPassword) {
      errors.confirmRootPassword = "Root passwords do not match";
    }

    setFormErrors(errors);
    return Object.keys(errors).length === 0;
  };

  const handleChange = (field: keyof FormData) => (event: React.ChangeEvent<HTMLInputElement>) => {
    const value = event.target.value;
    setFormData((previous) => ({ ...previous, [field]: value }));
    if (formErrors[field]) {
      setFormErrors((previous) => ({ ...previous, [field]: undefined }));
    }
  };

  const handleBack = () => {
    setStep((previous) => (previous === 1 ? previous : ((previous - 1) as 1 | 2 | 3)));
  };

  const handleNext = () => {
    if (step === 1 && validateStep1()) {
      setStep(2);
      return;
    }
    if (step === 2 && validateStep2()) {
      setStep(3);
    }
  };

  const handleSubmit = (event: React.FormEvent) => {
    event.preventDefault();
    if (!validateStep3()) {
      return;
    }

    dispatch(
      submitSetup({
        user: formData.username,
        password: formData.password,
        root_password: formData.rootPassword,
        email: formData.email || undefined,
      }),
    );
  };

  if (setupComplete) {
    return (
      <AuthSplitLayout
        title="Setup complete"
        description="Your server is configured and ready for login."
        panelTitle="Server ready"
        panelDescription="Core admin credentials are configured. Continue to login and start managing your workspace."
        panelFootnote="Initialization"
      >
        <div className="space-y-5">
          <div className="mx-auto flex size-14 items-center justify-center rounded-full bg-brand-subtle">
            <Check className="size-7 text-brand" />
          </div>

          <div className="rounded-xl border bg-muted/50 p-4">
            <p className="text-sm text-muted-foreground">
              DBA account <span className="font-semibold text-foreground">{createdUsername}</span> created successfully.
            </p>
            <p className="mt-2 text-sm text-muted-foreground">Root password has been configured.</p>
          </div>

          <p className="text-sm text-muted-foreground">Redirecting to login in 3 seconds.</p>
          <Button
            variant="brand"
            className={cn(authButtonClass, "w-full")}
            onClick={() => navigate("/login", { replace: true })}
          >
            Go to login
          </Button>
        </div>
      </AuthSplitLayout>
    );
  }

  return (
    <AuthSplitLayout
      title={STEP_TITLES[step]}
      description={STEP_DESCRIPTIONS[step]}
      panelTitle="First-run configuration"
      panelDescription="Create your initial DBA account and secure root access before opening the admin interface."
      panelFootnote="Setup wizard"
    >
      <form
        onSubmit={
          step === 3
            ? handleSubmit
            : (event) => {
                event.preventDefault();
                handleNext();
              }
        }
        className="space-y-6"
      >
        <div className="space-y-2">
          <div className="flex items-center justify-between text-xs text-muted-foreground">
            <span>Step {step} of 3</span>
            <span>{STEP_TITLES[step]}</span>
          </div>
          <div className="grid grid-cols-3 gap-2">
            {[1, 2, 3].map((value) => (
              <div key={value} className={cn("h-1.5 rounded-full", value <= step ? "bg-brand" : "bg-muted")} />
            ))}
          </div>
        </div>

        {step === 1 && (
          <div className="space-y-4">
            <Field data-invalid={Boolean(formErrors.username)}>
              <FieldLabel htmlFor="username" className="gap-1 text-sm">
                Username <span className="text-destructive">*</span>
              </FieldLabel>
              <Input
                id="username"
                value={formData.username}
                onChange={handleChange("username")}
                placeholder="admin"
                autoFocus
                aria-invalid={Boolean(formErrors.username)}
                className={authInputClass}
              />
              <FieldError>{formErrors.username}</FieldError>
            </Field>

            <Field data-invalid={Boolean(formErrors.email)}>
              <FieldLabel htmlFor="email" className="gap-1 text-sm">
                Email <span className="text-muted-foreground">(optional)</span>
              </FieldLabel>
              <Input
                id="email"
                type="email"
                value={formData.email}
                onChange={handleChange("email")}
                placeholder="admin@example.com"
                aria-invalid={Boolean(formErrors.email)}
                className={authInputClass}
              />
              <FieldError>{formErrors.email}</FieldError>
            </Field>
          </div>
        )}

        {step === 2 && (
          <div className="space-y-4">
            <Field data-invalid={Boolean(formErrors.password)}>
              <FieldLabel htmlFor="password" className="gap-1 text-sm">
                Password <span className="text-destructive">*</span>
              </FieldLabel>
              <div className="relative">
                <Input
                  id="password"
                  type={showPassword ? "text" : "password"}
                  value={formData.password}
                  onChange={handleChange("password")}
                  placeholder="Enter password"
                  autoFocus
                  aria-invalid={Boolean(formErrors.password)}
                  className={cn(authInputClass, "pr-11")}
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  onClick={() => setShowPassword((previous) => !previous)}
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground"
                  aria-label={showPassword ? "Hide password" : "Show password"}
                  aria-pressed={showPassword}
                >
                  {showPassword ? <EyeOff className="size-4" /> : <Eye className="size-4" />}
                </Button>
              </div>
              <FieldError>{formErrors.password}</FieldError>
            </Field>

            <Field data-invalid={Boolean(formErrors.confirmPassword)}>
              <FieldLabel htmlFor="confirmPassword" className="gap-1 text-sm">
                Confirm Password <span className="text-destructive">*</span>
              </FieldLabel>
              <Input
                id="confirmPassword"
                type={showPassword ? "text" : "password"}
                value={formData.confirmPassword}
                onChange={handleChange("confirmPassword")}
                placeholder="Confirm password"
                aria-invalid={Boolean(formErrors.confirmPassword)}
                className={authInputClass}
              />
              <FieldError>{formErrors.confirmPassword}</FieldError>
            </Field>
          </div>
        )}

        {step === 3 && (
          <div className="space-y-4">
            <Alert className="bg-muted/50">
              <AlertCircle />
              <AlertTitle>Security notice</AlertTitle>
              <AlertDescription>
                The root password is used for system administration. Store it securely.
              </AlertDescription>
            </Alert>

            <Field data-invalid={Boolean(formErrors.rootPassword)}>
              <FieldLabel htmlFor="rootPassword" className="gap-1 text-sm">
                Root Password <span className="text-destructive">*</span>
              </FieldLabel>
              <div className="relative">
                <Input
                  id="rootPassword"
                  type={showRootPassword ? "text" : "password"}
                  value={formData.rootPassword}
                  onChange={handleChange("rootPassword")}
                  placeholder="Enter root password"
                  autoFocus
                  aria-invalid={Boolean(formErrors.rootPassword)}
                  className={cn(authInputClass, "pr-11")}
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  onClick={() => setShowRootPassword((previous) => !previous)}
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground"
                  aria-label={showRootPassword ? "Hide root password" : "Show root password"}
                  aria-pressed={showRootPassword}
                >
                  {showRootPassword ? <EyeOff className="size-4" /> : <Eye className="size-4" />}
                </Button>
              </div>
              <FieldError>{formErrors.rootPassword}</FieldError>
            </Field>

            <Field data-invalid={Boolean(formErrors.confirmRootPassword)}>
              <FieldLabel htmlFor="confirmRootPassword" className="gap-1 text-sm">
                Confirm Root Password <span className="text-destructive">*</span>
              </FieldLabel>
              <Input
                id="confirmRootPassword"
                type={showRootPassword ? "text" : "password"}
                value={formData.confirmRootPassword}
                onChange={handleChange("confirmRootPassword")}
                placeholder="Confirm root password"
                aria-invalid={Boolean(formErrors.confirmRootPassword)}
                className={authInputClass}
              />
              <FieldError>{formErrors.confirmRootPassword}</FieldError>
            </Field>
          </div>
        )}

        {error && (
          <Alert variant="destructive" className="border-destructive/20 bg-destructive/5">
            <AlertCircle />
            <AlertTitle>Setup failed</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        <div className="flex gap-3">
          {step > 1 && (
            <Button type="button" variant="outline" className={cn(authButtonClass, "flex-1")} onClick={handleBack}>
              Back
            </Button>
          )}
          {step < 3 ? (
            <Button type="submit" variant="brand" className={cn(authButtonClass, "flex-1")}>
              Continue
            </Button>
          ) : (
            <Button
              type="submit"
              variant="brand"
              className={cn(authButtonClass, "flex-1")}
              disabled={isSubmitting}
            >
              {isSubmitting ? (
                <>
                  <Spinner data-icon="inline-start" />
                  Setting up...
                </>
              ) : (
                "Complete setup"
              )}
            </Button>
          )}
        </div>
      </form>
    </AuthSplitLayout>
  );
}
