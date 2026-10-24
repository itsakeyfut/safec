void *malloc(int n);
int grow(int **pp);

int main(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    grow(&a);
    grow(&a);
    return 0;
}
