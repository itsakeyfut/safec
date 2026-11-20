static int *a = (int *)0;
char *b = ((void *)0);
int main(void) {
    return a != 0 || b != 0;
}
