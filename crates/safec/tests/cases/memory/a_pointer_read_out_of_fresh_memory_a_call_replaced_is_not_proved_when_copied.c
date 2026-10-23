void *malloc(int n);
void free(void *p);
void fill(int **slot);

int f(void) {
    int **box = malloc(8);
    if (box == 0) {
        return 1;
    }
    int *t = *box;
    fill(&t);
    int *u = t;
    if (u != 0) {
        *u = 1;
    }
    free(box);
    return 0;
}
