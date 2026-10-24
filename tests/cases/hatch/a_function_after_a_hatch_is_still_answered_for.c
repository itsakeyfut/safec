__attribute__((annotate("safec_unchecked")))
int raw(int *p) {
    return *p;
}

void free(void *p);
void poke(int *p);
void take(int * _Nonnull p);

int checked(int *p, int *q) {
    int x = *q;
    int *r = p;
    poke(r);
    take(p);
    free(p);
    return x;
}
