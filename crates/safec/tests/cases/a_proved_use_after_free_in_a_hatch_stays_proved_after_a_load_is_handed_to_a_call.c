void *malloc(int n);
void free(void *p);
void g(int *p);

__attribute__((annotate("safec_unchecked")))
int read_after_free(void) {
    int **tab = malloc(8);
    if (tab == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    *p = 1;
    *tab = p;
    free(p);
    int *q = *tab;
    g(q);
    return *p;
}

int main(void) { return read_after_free(); }
