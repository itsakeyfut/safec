void *malloc(int n);
void free(void *p);

int main(void) {
    int *slot = 0;
    int **t2 = &slot;
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    p[0] = 1;
    *t2 = p;
    free(p);
    return **t2;
}
