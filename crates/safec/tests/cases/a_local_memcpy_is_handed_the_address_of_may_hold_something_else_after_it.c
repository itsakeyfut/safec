void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);

int main(void) {
    int *q = malloc(4);
    int *p = malloc(4);
    memcpy(&q, &p, 8);
    free(q);
    free(p);
    return 0;
}
