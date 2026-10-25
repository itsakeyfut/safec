void *malloc(int n);
void g(void *q);

int main(void) {
    int *p = malloc(4);
    if (p == 0) {
        return 1;
    }
    g(p + 5);
    return 0;
}
