/*
 * The sum of the first eleven Fibonacci numbers, kept in a heap cell.
 *
 * It uses what safec compiles today: functions and recursion, `if` and
 * `for`, pointers, `malloc` and `free`, and a test against null. safec
 * checks its memory as it compiles it, and this program passes.
 *
 * From the repository root, with `clang` on PATH to link the program:
 *
 *     cargo run -- examples/fib.c -o fib
 *     ./fib
 *
 * On Windows, name it `-o fib.exe` and run `.\fib.exe`.
 *
 * It prints nothing and exits with status 143: there are no string
 * literals yet, so the answer is the exit status. Without `clang`, the
 * checks still run:
 *
 *     cargo run -- examples/fib.c --emit safety-ir -o fib.ir
 *
 * Then break it. Each line marked BREAK IT below says what to change and
 * which error safec reports for it. The line marked NOT CAUGHT is a defect
 * safec does not see yet.
 */

void *malloc(int n);
void free(void *p);

int fib(int n) {
    if (n < 2) {
        return n;
    }
    return fib(n - 1) + fib(n - 2);
}

int main(void) {
    int *total = malloc(4);
    // BREAK IT (SC0403): delete this null test.
    if (total == 0) {
        return 1;
    }
    // NOT CAUGHT: delete this line and it still builds.
    *total = 0;
    for (int i = 0; i <= 10; i++) {
        *total += fib(i);
    }
    int result = *total;
    // BREAK IT (SC0402): move this free above the line before.
    // BREAK IT (SC0401): write this free twice.
    free(total);
    return result;
}
