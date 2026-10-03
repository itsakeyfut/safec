/* The phase 1 target: the smallest program with a call in it. */

int add(int a, int b) {
    return a + b;
}

int main(void) {
    return add(1, 2);
}
