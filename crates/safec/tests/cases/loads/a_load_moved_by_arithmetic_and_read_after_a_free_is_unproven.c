void *malloc(int n);
void free(void *p);
void release(int *p);

int main(void) {
    int ***t3 = malloc(16);
    if (t3 == 0) {
        return 0;
    }
    int **t2 = malloc(16);
    if (t2 == 0) {
        return 0;
    }
    int *p = malloc(8);
    if (p == 0) {
        return 0;
    }
    p[0] = 1;
    *t2 = p;
    *t3 = t2;
    int i = 0;
    free(p);
    return *(**t3 + i);
}
