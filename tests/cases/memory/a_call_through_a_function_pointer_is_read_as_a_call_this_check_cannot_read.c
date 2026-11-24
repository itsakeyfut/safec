void *malloc(unsigned long long);
void free(void *);

struct ops {
    int (*get)(void);
};

int *global;
int (*kept)(void);

void take(int (*f)(void));
void use(char *p);

__attribute__((annotate("safec_unchecked"))) void hatch(void) {
    free(global);
}

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

int freed_callee_through_a_member(void) {
    struct ops *o = malloc(sizeof *o);
    if (!o)
        return 0;
    o->get = zero;
    free(o);
    return o->get();
}

int unassigned_member_called(void) {
    struct ops o;
    return o.get();
}

int unassigned_called_through_a_pointer_to_it(void) {
    int (*f)(void);
    int (**pp)(void) = &f;
    return (*pp)();
}

int apply(int (*f)(int *), int *p) {
    if (!f)
        return 0;
    return f(p);
}

int nonnull_as_an_argument(void) {
    int x = 1;
    return apply(read_it, &x);
}

int nonnull_as_a_condition(void) {
    if (read_it)
        return 1;
    return 0;
}

int nonnull_compared(int (*g)(int *)) {
    return g == read_it;
}

int nonnull_negated(void) {
    return !read_it;
}

int nonnull_converted(void) {
    int (*g)(void) = (int (*)(void))read_it;
    return g != 0;
}

int through_a_pointer_a_hatch_may_free_behind(void (*f)(void)) {
    int *p = malloc(4);
    if (!p)
        return 0;
    *p = 1;
    if (!f)
        return 0;
    f();
    return *p;
}

int stored_and_read_as_data(void) {
    int (*f)(void);
    int (**pp)(void) = &f;
    *pp = zero;
    int *q = (int *)*pp;
    return *q;
}

int stored_in_a_member_and_read_as_data(void) {
    struct ops o;
    o.get = zero;
    int *q = (int *)o.get;
    return *q;
}

int a_literal_handed_through_a_pointer(void (*f)(char *)) {
    if (!f)
        return 0;
    f("literal");
    return 0;
}

int called_twice_through_a_pointer_that_may_be_null(int c) {
    int (*f)(void) = 0;
    if (c)
        f = zero;
    f();
    return f();
}

int unassigned_where_a_call_through_a_pointer_returns(int c, void (*f)(void)) {
    int x = 1;
    int *p;
    if (c)
        p = &x;
    else if (f)
        f();
    return *p;
}

int handed_a_narrow_block_through_a_pointer(void (*f)(int *), void (*g)(void *)) {
    char c = 0;
    int *p = (int *)&c;
    if (!f || !g)
        return 0;
    g(p);
    f(p);
    return 0;
}

int (*callback_leaves(int c))(void) {
    int (*f)(void) = zero;
    kept = f;
    take(f);
    return c ? zero : f;
}
