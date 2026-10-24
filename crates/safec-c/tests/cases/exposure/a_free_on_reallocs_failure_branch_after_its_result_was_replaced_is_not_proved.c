void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);
void fill(int **pp);

int main(void) {
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    p[0] = 1;
    int *q = realloc(p, 8);
    fill(&q);
    if (q == 0) {
        free(p);
        return 0;
    }
    return 0;
}
