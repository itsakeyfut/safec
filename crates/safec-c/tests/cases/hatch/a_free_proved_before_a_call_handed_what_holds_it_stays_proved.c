void *malloc(int n);
void free(void *p);
void g(int **b);

__attribute__((annotate("safec_unchecked")))
int f(void) {
    int **box = malloc(8);
    int *p = malloc(4);
    if (box == 0) {
        return 0;
    }
    if (p == 0) {
        return 0;
    }
    *box = p;
    free(p);
    g(box);
    return *p;
}
