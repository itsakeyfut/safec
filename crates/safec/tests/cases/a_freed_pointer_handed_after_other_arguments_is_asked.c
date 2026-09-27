void *malloc(int n);
void free(void *p);
int mix(int n, int *p, int *q);

int main(void) {
    int *a = malloc(4);
    int *b = malloc(4);
    if (a == 0) {
        return 0;
    }
    if (b == 0) {
        return 0;
    }
    free(b);
    return mix(1, a, b);
}
