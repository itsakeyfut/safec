struct S {
    int a;
};

int certainly(void) {
    struct S *p = 0;
    return p->a;
}

int maybe(struct S *p) {
    if (p) {
    }
    return p->a;
}

int main(void) {
    struct S s;
    s.a = 0;
    return certainly() + maybe(&s);
}
