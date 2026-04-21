# Specifications: Recovery Code Mechanics

## 1. Code Generation
- Format: `TCHN-XXXXX-XXXXX` (Alphanumeric, uppercase).
- Quantity: 10 codes per user.
- Storage: ONLY the hashed versions of these codes must be saved in the `system-faas-auth` state store. The plaintext codes are returned only once in the HTTP response during setup.

## 2. Code Consumption
- Add an endpoint/function: `consume-recovery-code(username, code) -> result<temp-session, error>`.
- If the hash of the provided code matches one in the database, the code is deleted from the array (burned), and the user is granted an emergency session to reconfigure their 2FA settings.

## 3. UI First-Run Enhancements
- In the "First-Run Interceptor" modal, after scanning the TOTP QR Code, insert a new mandatory step: "Save your Recovery Codes".
- Present a `.txt` file download button (`tachyon-recovery-codes.txt`). Do not allow the user to click "Complete Setup" until they have either downloaded the file or copied the text to their clipboard.