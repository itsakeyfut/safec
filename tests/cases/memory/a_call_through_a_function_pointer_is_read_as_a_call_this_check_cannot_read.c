void *malloc(unsigned long long);
void free(void *);

struct ops {
    int (*get)(void);
};

int zero(void) {
    return 0;
}

int read_it(int *_Nonnull p) {
    return *p;
}

int stored_and_called(void) {
    int (*f)(void) = zero;
    return f() + (*zero)() + (f == zero);
}

int freed_through_a_pointer(void) {
    void (*release)(void *) = free;
    int *p = malloc(4);
    if (!p)
        return 0;
    *p = 1;
    release(p);
    return *p;
}

int read_as_data(void) {
    int *q = (int *)zero;
    return *q;
}

int through_a_pointer_that_may_be_null(int c) {
    int (*f)(void) = 0;
    if (c)
        f = zero;
    return f();
}

int through_an_unassigned_pointer(void) {
    int (*f)(void);
    return f();
}

int nonnull_address_taken(void) {
    int (*f)(int *) = read_it;
    int x = 1;
    return f(&x);
}

int converted(void) {
    void (*f)(void) = (void (*)(void))zero;
    f();
    return 0;
}

int in_a_member(void) {
    struct ops ops;
    ops.get = zero;
    int (*f)(void) = ops.get;
    if (!f)
        return 0;
    return f();
}
