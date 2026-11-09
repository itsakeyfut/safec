void *malloc(int n);
void *calloc(int n, int m);
void free(void *p);
int **f(void) {
    int ***box = calloc(1, 8);
    if (!box) return 0;
    for (;;) {
        int **b = malloc(8);
        if (!b) {
            int **old = *box;
            free(box);
            if (!old) return 0;
            return old;
        }
        *box = b;
    }
}
