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

int address_alone(struct S *p) {
    int *q = &p->a;
    return q != 0;
}

int main(void) {
    struct S s;
    s.a = 0;
    return certainly() + maybe(&s) + address_alone(0);
}
