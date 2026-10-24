void *malloc(int n);
void free(void *p);
int use2(int **pp);

int main(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    free(a);
    a = 0;
    return use2(&a);
}
