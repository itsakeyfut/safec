int sum(int n, ...) {
    __builtin_va_list a;
    __builtin_va_start(a, n);
    int s = 0;
    for (int i = 0; i < n; i++)
        s += __builtin_va_arg(a, int);
    __builtin_va_end(a);
    return s;
}

// A pointer and a `long long` past the parameters, and a copy of the list
// that reads on from where the first had got to.
long long mixed(int n, ...) {
    __builtin_va_list a;
    __builtin_va_list b;
    __builtin_va_start(a, n);
    int *p = __builtin_va_arg(a, int *);
    __builtin_va_copy(b, a);
    long long big = __builtin_va_arg(a, long long);
    long long again = __builtin_va_arg(b, long long);
    int last = __builtin_va_arg(b, int);
    __builtin_va_end(b);
    __builtin_va_end(a);
    if (!p)
        return -1;
    return *p + big + again + last;
}

int main(void) {
    int wrong = 0;
    int x = 3;
    wrong += sum(2, 1, -1) == 0 ? 0 : 1;
    wrong += sum(4, 1, 2, 3, 4) == 10 ? 0 : 1;
    wrong += mixed(0, &x, 1234567890123LL, 5) == 3 + 2 * 1234567890123LL + 5 ? 0 : 1;
    return wrong;
}
