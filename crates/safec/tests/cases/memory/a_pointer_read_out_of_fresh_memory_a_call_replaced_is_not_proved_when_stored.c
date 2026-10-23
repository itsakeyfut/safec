void *malloc(int n);
void free(void *p);

void fill(int **slot, int *to) {
    *slot = to;
}

int f(void) {
    int q = 0;
    int **box = malloc(8);
    if (box == 0) {
        return 1;
    }
    int **h = malloc(8);
    if (h == 0) {
        free(box);
        return 1;
    }
    int *t = *box;
    fill(&t, &q);
    *h = t;
    int *u = *h;
    *u = 1;
    free(h);
    free(box);
    return 0;
}
