void *malloc(unsigned long long);
void free(void *);

int dereferenced(int n, ...) {
    __builtin_va_list a;
    __builtin_va_start(a, n);
    int *p = __builtin_va_arg(a, int *);
    __builtin_va_end(a);
    if (!p)
        return 0;
    return *p;
}
int null_tested_nowhere(int n, ...) {
    __builtin_va_list a;
    __builtin_va_start(a, n);
    int *p = __builtin_va_arg(a, int *);
    __builtin_va_end(a);
    return *p;
}
int integers_are_not_doubted(int n, ...) {
    __builtin_va_list a;
    __builtin_va_start(a, n);
    int x = __builtin_va_arg(a, int);
    __builtin_va_end(a);
    return x + n;
}
int before_the_end(int n, ...) {
    __builtin_va_list a;
    __builtin_va_start(a, n);
    int *p = __builtin_va_arg(a, int *);
    int v = 0;
    if (p)
        v = *p;
    __builtin_va_end(a);
    return v;
}

int after_free(int n, ...) {
    __builtin_va_list *pa = malloc(sizeof(__builtin_va_list));
    if (!pa)
        return 0;
    __builtin_va_start(*pa, n);
    free(pa);
    return __builtin_va_arg(*pa, int);
}

int through_null(int n, ...) {
    __builtin_va_list *pa = 0;
    return __builtin_va_arg(*pa, int);
}

int uninitialised_pointer(int n, ...) {
    __builtin_va_list *pa;
    return __builtin_va_arg(*pa, int);
}

int via_address(int n, ...) {
    __builtin_va_list a;
    int *p;
    int **pp = &p;
    __builtin_va_start(a, n);
    *pp = __builtin_va_arg(a, int *);
    __builtin_va_end(a);
    return *p;
}
