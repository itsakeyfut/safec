void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);

int main(void) {
    int *d = malloc(8);
    int *s = malloc(8);
    if (d == 0) {
        return 0;
    }
    if (s == 0) {
        return 0;
    }
    free(d);
    memcpy(d, s, 8);
    free(s);
    return 0;
}
