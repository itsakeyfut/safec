void *malloc(int n);
int use2(int **pp);

int main(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    return use2(&a);
}
