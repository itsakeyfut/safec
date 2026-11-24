int k(int a) {
    return a;
}

int zero(void) {
    return 0;
}

void wr(int *p);
int callit(int (*f)(int *));

int initialized(void) {
    int (*g)(int *) = k;
    return 0;
}

int passed(void) {
    wr(zero);
    return callit(k);
}

int (*returned(void))(int *) {
    return k;
}

int as_an_integer(void) {
    int i;
    i = zero;
    return i;
}

int through_void(void) {
    void *v = zero;
    int (*f)(void) = v;
    return 0;
}

int its_own_type(void) {
    int (*f)(int) = k;
    int (*g)(int) = &k;
    int (*h)(int) = *k;
    void *v = &f;
    return f(1) + g(1) + h(1) + (v != 0);
}
