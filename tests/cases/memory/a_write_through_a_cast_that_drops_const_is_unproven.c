const int g = 1;
int main(void) {
    *(int *)&g = 2;
    return g;
}
