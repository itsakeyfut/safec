void *malloc(int n);
void *memcpy(void *d, void *s, int n);

int main(void) {
    int *s = 0;
    int **b = malloc(8);
    if (b == 0) {
        return 0;
    }
    memcpy(&s, b, 8);
    int *r = s;
    if (r == 0) {
        return 0;
    }
    *r = 1;
    return 0;
}
