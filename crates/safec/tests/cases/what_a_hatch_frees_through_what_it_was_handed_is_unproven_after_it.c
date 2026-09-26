void *malloc(int n);
void free(void *p);

__attribute__((annotate("safec_unchecked")))
void drop_inner(int **box) {
    free(*box);
}

int main(void) {
    int **box = malloc(8);
    if (box == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    *box = p;
    drop_inner(box);
    int r = *p;
    return r;
}
