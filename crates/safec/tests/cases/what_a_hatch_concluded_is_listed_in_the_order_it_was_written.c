void free(void *p);
void poke(int *p);

__attribute__((annotate("safec_unchecked")))
int raw(int *p, int *q) {
    int x = *q;
    int *r = p;
    poke(r);
    free(p);
    return x;
}
