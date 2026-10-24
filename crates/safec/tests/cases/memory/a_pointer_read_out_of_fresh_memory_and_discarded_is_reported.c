void *malloc(int n);
void free(void *p);

int f(int *q) {
    int **box = malloc(8);
    if (box == 0) {
        return 1;
    }
    int *t = *box;
    free(box);
    return 0;
}
