static int sum(int *values, int count) {
    int total = 0;
    for (int i = 0; i < count; i++) {
        total += values[i];
    }
    return total;
}

static int sized(int n) {
    int a[n];
    a[0] = 1;
    a[n - 1] = 2;
    // Sized when it runs: `n` elements of `int`.
    return a[0] + a[n - 1] + (int)sizeof(a);
}

int main(void) {
    int wrong = 0;
    wrong += sized(3) == 3 + 3 * (int)sizeof(int) ? 0 : 1;

    int n = 5;
    int a[n];
    for (int i = 0; i < n; i++) {
        a[i] = i;
    }
    wrong += sum(a, n) == 10 ? 0 : 1;

    // A fresh array each time round, its storage given back at the end of
    // each pass: 200000 passes of 4096 bytes would be 800 MB of stack
    // otherwise.
    long long seen = 0;
    for (int i = 0; i < 200000; i++) {
        int count = 4096;
        char buffer[count];
        buffer[0] = (char)i;
        buffer[count - 1] = 1;
        seen += buffer[count - 1];
    }
    wrong += seen == 200000 ? 0 : 1;
    return wrong;
}
