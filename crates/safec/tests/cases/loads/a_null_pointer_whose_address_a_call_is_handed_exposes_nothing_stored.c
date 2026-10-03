void *malloc(int n);
void escape(int **q);

int main(void) {
    int **a = malloc(8);
    if (a == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    *a = p;
    int *z = 0;
    escape(&z);
    return *p;
}
